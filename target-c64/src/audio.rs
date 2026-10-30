//! Foreground DLT5 producer. The fixed-cycle assembly callback owns decoding.
//! IRQs stay disabled; READY is published only after its buffers are complete.
#![allow(non_upper_case_globals)]

use core::ptr::{addr_of, addr_of_mut, read_unaligned, read_volatile, write_unaligned, write_volatile};
use crate::platform::{self, RING_ADDRESS, SAMPLE_RATE};

pub const FREE: u8 = 0;
pub const READY: u8 = 1;
pub const PLAYING: u8 = 2;
pub const DONE: u8 = 3;
const INPUT: u16 = 0xe600;
const BOOK: u16 = 0xe680;
const CHUNK_PAGE: [u8; 6] = [0xc0, 0xc3, 0xc6, 0xc9, 0xe0, 0xe3];
/// Published PCM stays at most this far ahead of the sampler: 256 ms, so a cut
/// is heard within a quarter second. Six 128-frame chunk slots add 48 ms.
pub const LEAD_FRAMES: u32 = 4096;
/// Boot primes the ring to the lead less one chunk before arming the sampler.
pub const PRIME_FRAMES: u32 = LEAD_FRAMES - 128;
/// The outgoing song fades over its final 16384 frames (1.02 s); a manual skip
/// moves the audible end to one fade ahead, so it fades like an ending.
pub const FADE_FRAMES: u32 = 16384;

/// Byte-exact MOS C layout; never take references to the packed fields.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct AudioTrack {
    pub address: u32,
    pub frames: u32,
    pub stream: u32,
    pub books: u16,
    pub short_segment: u16,
    pub long_segments: u16,
    pub id: u8,
}
impl AudioTrack {
    pub const EMPTY: Self = Self { address: 0, frames: 0, stream: 0, books: 0,
        short_segment: 0, long_segments: 0, id: 0 };
}
const _: () = assert!(core::mem::size_of::<AudioTrack>() == 19);
const _: () = assert!(core::mem::align_of::<AudioTrack>() == 1);

#[used] #[no_mangle] pub static mut havoc_audio_chunk_state: [u8; 6] = [0; 6];
#[used] #[no_mangle] pub static mut havoc_audio_chunk_units: [u8; 6] = [0; 6];
#[used] #[no_mangle] pub static mut havoc_audio_chunk_reset: [u8; 6] = [0; 6];
#[used] #[no_mangle] pub static mut havoc_audio_codes_hi: [u8; 6] = [0; 6];
#[used] #[no_mangle] pub static mut havoc_audio_pcm_hi: [u8; 6] = [0; 6];
#[used] #[no_mangle] pub static mut havoc_audio_lut_hi: [u8; 6] = [0; 6];
#[used] #[no_mangle] pub static mut audio_decode_track: AudioTrack = AudioTrack::EMPTY;
#[used] #[no_mangle] pub static mut havoc_fault: u8 = 0;
#[used] #[no_mangle] pub static mut audio_running: u8 = 0;
#[used] #[no_mangle] pub static mut audio_audible_track: u8 = 0;
#[used] #[no_mangle] pub static mut havoc_underruns: u16 = 0;
#[used] #[no_mangle] pub static mut havoc_minimum_lead: u16 = 32767;
#[used] #[no_mangle] pub static mut havoc_dma_ticks: u16 = 0;
#[used] #[no_mangle] pub static mut produced: u32 = 0;
#[used] #[no_mangle] pub static mut consumed: u32 = 0;
#[used] #[no_mangle] pub static mut audio_audible_start: u32 = 0;
#[used] #[no_mangle] pub static mut audio_audible_frames: u32 = 0;
#[used] #[no_mangle] pub static mut audio_event: u8 = 0;
#[used] #[no_mangle] pub static mut audio_next_valid: u8 = 0;
/// Sampler level (0..=32) the foreground applies each frame; 32 is full volume.
#[used] #[no_mangle] pub static mut audio_fade: u8 = 32;

static mut queued: AudioTrack = AudioTrack::EMPTY;
static mut chunk_lut: [u8; 6] = [0; 6];
static mut current_lut: u8 = 0;
#[used] #[no_mangle] static mut fill_index: u8 = 0;
#[used] #[no_mangle] static mut drain_index: u8 = 0;
static mut reset_next: u8 = 0;
static mut boundary_pending: u8 = 0;
static mut boundary_id: u8 = 0;
static mut segment: u16 = 0;
static mut segment_left: u16 = 0;
static mut source_frame: u32 = 0;
static mut stream_position: u32 = 0;
#[used] #[no_mangle] static mut planned: u32 = 0;
static mut last_tick: u32 = 0;
static mut clock_remainder: u32 = 0;
static mut boundary: u32 = 0;
static mut boundary_frames: u32 = 0;
#[used] #[no_mangle] static mut drain_cost: u16 = 280;
#[used] #[no_mangle] static mut fill_cost: u16 = 170;
#[used] #[no_mangle] static mut book_fill_cost: u16 = 250;

extern "C" {
    pub fn havoc_raster_work();
    fn audio_unpack_fast(argument: u16);
    fn audio_lut_fast(page: u8);
    fn audio_timer16() -> u16;
}

macro_rules! get { ($name:ident) => { read_volatile(addr_of!($name)) }; }
macro_rules! set { ($name:ident, $value:expr) => { write_volatile(addr_of_mut!($name), $value) }; }
macro_rules! item {
    ($name:ident, $index:expr) => {
        read_volatile(addr_of!($name).cast::<u8>().add($index as usize))
    };
}
macro_rules! put_item {
    ($name:ident, $index:expr, $value:expr) => {
        write_volatile(addr_of_mut!($name).cast::<u8>().add($index as usize), $value)
    };
}

#[inline(always)]
unsafe fn page(index: u8) -> u8 {
    // Every caller holds a slot/table index below six. A FREE slot guarantees
    // the live-LUT mask has at least one free bit among these six tables.
    *CHUNK_PAGE.as_ptr().add(index as usize)
}

#[inline(always)]
unsafe fn le32(address: u16) -> u32 {
    platform::read(address) as u32
        | ((platform::read(address.wrapping_add(1)) as u32) << 8)
        | ((platform::read(address.wrapping_add(2)) as u32) << 16)
        | ((platform::read(address.wrapping_add(3)) as u32) << 24)
}

/// Bounded quotients avoid a generic resident 32-bit division helper.
#[inline(never)]
#[link_section = ".tailcode.audio_divide_bounded"]
pub(crate) fn divide_bounded(remainder: &mut u32, mut trial: u32, mut bits: u8) -> u16 {
    let mut quotient = 0u16;
    loop {
        quotient = quotient.wrapping_shl(1);
        if *remainder >= trial {
            *remainder -= trial;
            quotient = quotient.wrapping_add(1);
        }
        trial >>= 1;
        bits -= 1;
        if bits == 0 { return quotient; }
    }
}

#[no_mangle]
pub unsafe extern "C" fn audio_track_read(out: *mut AudioTrack, address: u32, bytes: u32, id: u8) -> u8 {
    platform::reu_fetch(address, platform::ram_ptr(INPUT), 32);
    if platform::read(INPUT) != b'D' || platform::read(INPUT+1) != b'L'
        || platform::read(INPUT+2) != b'T' || platform::read(INPUT+3) != b'5'
        || le32(INPUT+4) != SAMPLE_RATE || platform::read(INPUT+16) != 8
        || platform::read(INPUT+17) != 0 || platform::read(INPUT+20) != 3
        || platform::read(INPUT+21) != 2 { return 0; }
    let frames = le32(INPUT+8);
    let books = platform::read(INPUT+18) as u16 | ((platform::read(INPUT+19) as u16) << 8);
    if frames == 0 || frames & 3 != 0 || books == 0 { return 0; }
    let stream = 32 + ((books as u32) << 5);
    let stream_bytes = (frames >> 2).wrapping_mul(3);
    if le32(INPUT+12) != stream_bytes
        || stream.wrapping_add(stream_bytes).wrapping_add(stream_bytes.wrapping_add(1023) >> 10) != bytes {
        return 0;
    }
    let mut groups = frames >> 2;
    let short_groups = divide_bounded(&mut groups, (books as u32) << 7, 8);
    if short_groups == 0 || groups >= books as u32 { return 0; }
    write_unaligned(out, AudioTrack { address, frames, stream, books,
        short_segment: short_groups << 2, long_segments: groups as u16, id });
    1
}

#[inline(never)]
#[link_section = ".lowcode.audio_source_reset"]
unsafe fn source_reset() {
    source_frame = 0; stream_position = 0; segment = 0;
    reset_next = 1; current_lut = 255;
    segment_left = audio_decode_track.short_segment
        + if audio_decode_track.long_segments != 0 { 4 } else { 0 };
}

#[no_mangle]
#[inline(never)]
#[link_section = ".disk_init.audio_init"]
pub unsafe extern "C" fn audio_init(track: *const AudioTrack) {
    audio_decode_track = read_unaligned(track);
    source_reset();
    let mut i = 0;
    while i < 6 {
        put_item!(havoc_audio_chunk_state, i, FREE);
        put_item!(havoc_audio_codes_hi, i, page(i));
        put_item!(havoc_audio_pcm_hi, i, page(i)+1);
        put_item!(havoc_audio_lut_hi, i, page(i)+2);
        i += 1;
    }
    i = 0;
    while i < 4 { platform::write(0xf0+i as u16, 0); i += 1; }
    platform::write(0xf4, 0); platform::write(0xf5, 0); platform::write(0xf6, 5);
    set!(audio_audible_track, audio_decode_track.id);
    set!(audio_audible_frames, audio_decode_track.frames);
    set!(audio_audible_start, 0); set!(produced, 0); set!(consumed, 0); planned = 0;
    fill_index = 0; drain_index = 0; set!(audio_next_valid, 0);
    boundary_pending = 0; set!(audio_event, 0); set!(audio_fade, 32);
}

/// Queues the next song. With `cut`, the audible song ends one fade from now
/// instead of at its natural end.
#[no_mangle]
pub unsafe extern "C" fn audio_queue(track: *const AudioTrack, cut: u8) {
    if get!(audio_next_valid) != 0 || boundary_pending != 0 { return; }
    queued = read_unaligned(track); set!(audio_next_valid, 1);
    if cut != 0 { audio_end_in(FADE_FRAMES); }
}

/// Ends the audible song no later than `frames` from now: it fades out and the
/// decoder switches within one chunk of that end. Ignored once a switch is pending.
#[no_mangle]
pub unsafe extern "C" fn audio_end_in(frames: u32) {
    if boundary_pending != 0 { return; }
    let end = get!(consumed).wrapping_sub(get!(audio_audible_start)).wrapping_add(frames);
    if end < get!(audio_audible_frames) {
        set!(audio_audible_frames, end); audio_decode_track.frames = end;
    }
}

#[inline(never)]
unsafe fn prepare_chunk(slot: u8) {
    let mut table = current_lut;
    // A shortened end need not fall on a chunk boundary.
    if source_frame >= audio_decode_track.frames {
        if boundary_pending != 0 { set!(havoc_fault, 7); return; }
        if get!(audio_next_valid) != 0 {
            audio_decode_track = queued; set!(audio_next_valid, 0);
        }
        boundary = planned; boundary_frames = audio_decode_track.frames;
        boundary_id = audio_decode_track.id; boundary_pending = 1;
        source_reset(); table = 255;
    }
    if segment_left == 0 {
        segment += 1; table = 255;
        segment_left = audio_decode_track.short_segment
            + if segment < audio_decode_track.long_segments { 4 } else { 0 };
    }
    let frames = if segment_left > 128 { 128 } else { segment_left as u8 };
    let bytes = (frames >> 2)*3;
    if table == 255 {
        let mut used = 0u8;
        let mut i = 0;
        while i < 6 {
            if item!(havoc_audio_chunk_state, i) != FREE {
                used |= 1 << item!(chunk_lut, i);
            }
            i += 1;
        }
        table = 0;
        while used & 1 != 0 { used >>= 1; table += 1; }
        platform::reu_fetch(audio_decode_track.address+32+((segment as u32) << 5), platform::ram_ptr(BOOK), 32);
        audio_lut_fast(page(table)+2);
    }
    current_lut = table; put_item!(chunk_lut, slot, table);
    put_item!(havoc_audio_lut_hi, slot, page(table)+2);
    platform::reu_fetch(audio_decode_track.address+audio_decode_track.stream+stream_position,
        platform::ram_ptr(INPUT), bytes as u16);
    audio_unpack_fast(((bytes as u16) << 8) | item!(havoc_audio_codes_hi, slot) as u16);
    source_frame += frames as u32; planned = planned.wrapping_add(frames as u32);
    stream_position += bytes as u32; segment_left -= frames as u16;
    put_item!(havoc_audio_chunk_units, slot, frames >> 1);
    put_item!(havoc_audio_chunk_reset, slot, reset_next); reset_next = 0;
    put_item!(havoc_audio_chunk_state, slot, READY);
}

#[no_mangle]
pub unsafe extern "C" fn audio_service(deadline: u32) {
    let begin = audio_timer16();
    let mut count = 0;
    while count < 6 && item!(havoc_audio_chunk_state, drain_index) == DONE {
        count += 1;
        let bytes = (item!(havoc_audio_chunk_units, drain_index) as u16) << 2;
        let at = (get!(produced) as u16).wrapping_shl(1);
        let mut first = bytes;
        let pcm = platform::ram_ptr((item!(havoc_audio_pcm_hi, drain_index) as u16) << 8);
        if get!(produced).wrapping_sub(get!(consumed)).wrapping_add(bytes as u32 >> 1) >= LEAD_FRAMES { break; }
        let start = audio_timer16();
        if deadline != 0 && (start.wrapping_sub(deadline as u16) as i16) <= drain_cost.wrapping_add(32) as i16 { break; }
        if at != 0 && at as u32+bytes as u32 > 65536 { first = 0u16.wrapping_sub(at); }
        platform::reu_stash(pcm, RING_ADDRESS+at as u32, first);
        if first < bytes { platform::reu_stash(pcm.add(first as usize), RING_ADDRESS, bytes-first); }
        set!(produced, get!(produced).wrapping_add(bytes as u32 >> 1));
        put_item!(havoc_audio_chunk_state, drain_index, FREE);
        drain_index += 1; if drain_index == 6 { drain_index = 0; }
        let elapsed = start.wrapping_sub(audio_timer16());
        if elapsed > drain_cost { drain_cost = elapsed; }
    }
    count = 0;
    while count < 6 && item!(havoc_audio_chunk_state, fill_index) == FREE {
        count += 1;
        let new_book = current_lut == 255 || segment_left == 0
            || source_frame >= audio_decode_track.frames;
        let cost = if new_book { book_fill_cost } else { fill_cost };
        let start = audio_timer16();
        if deadline != 0 && (start.wrapping_sub(deadline as u16) as i16) <= cost.wrapping_add(32) as i16 { break; }
        prepare_chunk(fill_index); if get!(havoc_fault) != 0 { return; }
        fill_index += 1; if fill_index == 6 { fill_index = 0; }
        let elapsed = start.wrapping_sub(audio_timer16());
        if new_book { if elapsed > book_fill_cost { book_fill_cost = elapsed; } }
        else if elapsed > fill_cost { fill_cost = elapsed; }
    }
    set!(havoc_dma_ticks, begin.wrapping_sub(audio_timer16()));
}

#[no_mangle]
pub unsafe extern "C" fn audio_clock() {
    let now = platform::timer_read();
    let elapsed = last_tick.wrapping_sub(now); last_tick = now;
    if get!(audio_running) == 0 { return; }
    if elapsed > 65535 { set!(havoc_fault, 6); platform::sampler_stop(); set!(audio_running, 0); return; }
    let mut scaled = (elapsed << 15).wrapping_add(clock_remainder);
    // Boot validates 1.8M..2.4M ticks/ring. The 11-bit trial and numerator fit u32.
    let ticks = read_volatile(addr_of!(platform::sampler_ticks_per_ring));
    set!(consumed, get!(consumed).wrapping_add(divide_bounded(&mut scaled, ticks << 10, 11) as u32));
    clock_remainder = scaled;
    let lead = get!(produced).wrapping_sub(get!(consumed));
    if lead <= 256 || lead >= 32768 {
        set!(havoc_underruns, get!(havoc_underruns).wrapping_add(1));
        set!(havoc_fault, 5); platform::sampler_stop(); set!(audio_running, 0); return;
    }
    if lead < get!(havoc_minimum_lead) as u32 { set!(havoc_minimum_lead, lead as u16); }
    if boundary_pending != 0 && get!(consumed).wrapping_sub(boundary) as i32 >= 0 {
        set!(audio_audible_track, boundary_id); set!(audio_audible_start, boundary);
        set!(audio_audible_frames, boundary_frames); boundary_pending = 0; set!(audio_event, 1);
    }
    // The audible song fades over its final FADE_FRAMES; the frame that crosses
    // the boundary already reports full volume for the incoming song.
    let remaining = audio_remaining();
    set!(audio_fade, if remaining >= FADE_FRAMES { 32 } else { ((remaining as u16) >> 9) as u8 });
    if get!(consumed) >= 0x01000000 {
        set!(consumed, get!(consumed).wrapping_sub(0x01000000));
        set!(produced, get!(produced).wrapping_sub(0x01000000));
        planned = planned.wrapping_sub(0x01000000);
        set!(audio_audible_start, get!(audio_audible_start).wrapping_sub(0x01000000));
        if boundary_pending != 0 { boundary = boundary.wrapping_sub(0x01000000); }
    }
}

#[no_mangle]
#[inline(never)]
#[link_section = ".tailcode.audio_remaining"]
pub unsafe extern "C" fn audio_remaining() -> u32 {
    let elapsed = get!(consumed).wrapping_sub(get!(audio_audible_start));
    if elapsed < get!(audio_audible_frames) { get!(audio_audible_frames)-elapsed } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn audio_start(_divider: u16) {
    set!(consumed, 0); clock_remainder = 0; platform::sampler_arm();
    last_tick = platform::timer_read(); set!(audio_running, 1);
}

/// Test-only view of the pending boundary: (pending, boundary).
#[cfg(test)]
pub(crate) unsafe fn test_boundary() -> (u8, u32) { (boundary_pending, boundary) }

/// Test-only rebasing adjustment; production's exported ABI remains unchanged.
#[cfg(test)]
pub(crate) unsafe fn test_rebase_shift(delta: u32) {
    set!(produced, get!(produced).wrapping_add(delta));
    set!(consumed, get!(consumed).wrapping_add(delta));
    planned = planned.wrapping_add(delta);
    set!(audio_audible_start, get!(audio_audible_start).wrapping_add(delta));
    if boundary_pending != 0 { boundary = boundary.wrapping_add(delta); }
}
