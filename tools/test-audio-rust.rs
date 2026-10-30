//! Native tests of the actual production Rust module. No C64U access.
#![allow(dead_code, non_upper_case_globals)]

use core::ptr::{addr_of, addr_of_mut, read_unaligned, read_volatile, write_volatile};

mod platform {
    use core::ptr::{addr_of_mut, copy_nonoverlapping};
    pub const SAMPLE_RATE: u32 = 16000;
    pub const RING_ADDRESS: u32 = 0xfe0000;
    pub static mut sampler_ticks_per_ring: u32 = 2097152; //64 ticks/sample
    static mut RAM: [u8; 65536] = [0; 65536];
    static mut REU: [u8; 0x1000000] = [0; 0x1000000];
    pub static mut TICKS: u32 = 0xffffffff;
    pub static mut TRANSFERS: u32 = 0;
    pub static mut STASHES: u32 = 0;
    pub static mut STOPS: u32 = 0;
    pub static mut ARMS: u32 = 0;
    pub unsafe fn ram_ptr(address: u16) -> *mut u8 {
        addr_of_mut!(RAM).cast::<u8>().add(address as usize)
    }
    unsafe fn reu_ptr(address: u32) -> *mut u8 {
        addr_of_mut!(REU).cast::<u8>().add(address as usize)
    }
    pub unsafe fn read(address: u16) -> u8 { *ram_ptr(address) }
    pub unsafe fn write(address: u16, value: u8) { *ram_ptr(address) = value; }
    pub unsafe fn timer_read() -> u32 { TICKS }
    pub unsafe fn sampler_arm() { ARMS += 1; }
    pub unsafe fn sampler_stop() { STOPS += 1; }
    pub unsafe fn reu_fetch(address: u32, destination: *mut u8, bytes: u16) {
        assert!(bytes != 0 && address+bytes as u32 <= 0x1000000);
        let ram_offset = destination as usize-ram_ptr(0) as usize;
        assert!(ram_offset+bytes as usize <= 65536);
        copy_nonoverlapping(reu_ptr(address), destination, bytes as usize);
        TRANSFERS += 1;
    }
    pub unsafe fn reu_stash(source: *const u8, address: u32, bytes: u16) {
        assert!(bytes != 0 && address >= RING_ADDRESS && address+bytes as u32 <= RING_ADDRESS+65536);
        let ram_offset = source as usize-ram_ptr(0) as usize;
        assert!(ram_offset+bytes as usize <= 65536);
        copy_nonoverlapping(source, reu_ptr(address), bytes as usize);
        STASHES += 1;
    }
    pub unsafe fn clear_ram() { core::ptr::write_bytes(ram_ptr(0), 0, 65536); }
    pub unsafe fn load(address: u32, data: &[u8]) { copy_nonoverlapping(data.as_ptr(), reu_ptr(address), data.len()); }
    pub unsafe fn peek_reu(address: u32) -> u8 { *reu_ptr(address) }
    pub unsafe fn poke_reu(address: u32, value: u8) { *reu_ptr(address) = value; }
}

#[path = "../target-c64/src/audio.rs"]
mod audio;

macro_rules! value { ($name:ident) => { read_volatile(addr_of!(audio::$name)) }; }
macro_rules! assign { ($name:ident, $value:expr) => { write_volatile(addr_of_mut!(audio::$name), $value) }; }
unsafe fn array(base: *const u8, index: u8) -> u8 { *base.add(index as usize) }
unsafe fn array_write(base: *mut u8, index: u8, value: u8) { *base.add(index as usize) = value; }

#[no_mangle]
pub unsafe extern "C" fn audio_timer16() -> u16 { platform::timer_read() as u16 }

#[no_mangle]
pub unsafe extern "C" fn audio_unpack_fast(argument: u16) {
    let page = (argument as u8 as u16) << 8;
    for group in 0..(argument >> 8)/3 {
        let at = 0xe600+group*3;
        let word = platform::read(at) as u32 | (platform::read(at+1) as u32) << 8
            | (platform::read(at+2) as u32) << 16;
        for frame in 0..4 { platform::write(page+group*8+frame*2, ((word >> (frame*6)) & 63) as u8); }
    }
}

#[no_mangle]
pub unsafe extern "C" fn audio_lut_fast(page: u8) {
    let base = (page as u16) << 8;
    for code in 0u16..64 {
        let left = 0xe680+(code & 7)*2;
        let right = 0xe690+(code >> 3)*2;
        platform::write(base+code, platform::read(left));
        platform::write(base+code+64, platform::read(left+1));
        platform::write(base+code+128, platform::read(right));
        platform::write(base+code+192, platform::read(right+1));
    }
}

/// Scalar callback models the documented assembly ABI, not instruction timing.
#[no_mangle]
pub unsafe extern "C" fn havoc_raster_work() {
    let mut slot = platform::read(0xf6);
    let quota = platform::read(0xf5).wrapping_sub(1);
    platform::write(0xf5, quota);
    let states = addr_of_mut!(audio::havoc_audio_chunk_state).cast::<u8>();
    if quota & 128 != 0 {
        platform::write(0xf5, 0);
        if array(states, slot) == audio::PLAYING { array_write(states, slot, audio::DONE); }
        slot = (slot+1) % 6;
        if array(states, slot) != audio::READY { return; }
        platform::write(0xf6, slot); array_write(states, slot, audio::PLAYING);
        platform::write(0xf5, array(addr_of!(audio::havoc_audio_chunk_units).cast(), slot));
        platform::write(0xf4, 0);
        if array(addr_of!(audio::havoc_audio_chunk_reset).cast(), slot) != 0 {
            for address in 0xf0..0xf4 { platform::write(address, 0); }
        }
        return;
    }
    for _ in 0..2 {
        let position = platform::read(0xf4) as u16;
        let code_page = array(addr_of!(audio::havoc_audio_codes_hi).cast(), slot) as u16;
        let code = platform::read((code_page << 8)+position) as u16;
        let lut = (array(addr_of!(audio::havoc_audio_lut_hi).cast(), slot) as u16) << 8;
        let pcm = (array(addr_of!(audio::havoc_audio_pcm_hi).cast(), slot) as u16) << 8;
        for channel in 0..2u16 {
            let predictor = 0xf0+channel*2;
            let previous = i16::from_le_bytes([platform::read(predictor), platform::read(predictor+1)]);
            let at = lut+channel*128+code;
            let delta = i16::from_le_bytes([platform::read(at), platform::read(at+64)]);
            let next = (previous as i32+delta as i32).clamp(-32768, 32767);
            platform::write(predictor, next as u8);
            platform::write(predictor+1, (next >> 8) as u8);
            platform::write(pcm+position+channel, ((next+128) >> 8).min(127) as u8);
        }
        platform::write(0xf4, (position as u8).wrapping_add(2));
    }
}

struct Asset { track: audio::AudioTrack, pcm: Vec<u8>, bytes: u32 }
const FRAME_COUNTS: [u32; 3] = [60000, 64004, 70004];
const BOOK_COUNTS: [u16; 3] = [180, 192, 209];
const ADDRESSES: [u32; 3] = [0x1000, 0x100000, 0x200000];
const DELTAS: [i16; 8] = [-32768, -128, -1, 0, 1, 127, 128, 32767];

fn source_code(track: usize, frame: u32) -> u8 { ((frame*13+frame/31+track as u32*7) & 63) as u8 }

/// Independent oracle reads the original packed bits, never the expanded LUT.
fn decode_prefix(blob: &[u8], count: u32) -> Vec<u8> {
    let frames = u32::from_le_bytes(blob[8..12].try_into().unwrap());
    let books = u16::from_le_bytes(blob[18..20].try_into().unwrap()) as u32;
    assert!(count <= frames);
    let stream = 32+books as usize*32;
    let short = (frames/4/books)*4;
    let long = (frames/4) % books;
    let split = long*(short+4);
    let mut predictors = [0i32; 2];
    let mut pcm = Vec::new();
    for frame in 0..count {
        let book = if frame < split { frame/(short+4) } else { long+(frame-split)/short };
        for channel in 0..2usize {
            let bit = (frame as usize*2+channel)*3;
            let code = (0..3).map(|offset| (((blob[stream+(bit+offset)/8] >> ((bit+offset) % 8)) & 1) as usize) << offset).sum::<usize>();
            let at = 32+book as usize*32+channel*16+code*2;
            let delta = i16::from_le_bytes([blob[at], blob[at+1]]);
            predictors[channel] = (predictors[channel]+delta as i32).clamp(-32768, 32767);
            pcm.push(((predictors[channel]+128) >> 8).min(127) as u8);
        }
    }
    pcm
}

unsafe fn assets() -> Vec<Asset> {
    let mut result = Vec::new();
    for track in 0..3 {
        let frames = FRAME_COUNTS[track];
        let books = BOOK_COUNTS[track];
        let stream = 32+books as usize*32;
        let stream_bytes = frames as usize/4*3;
        let mut blob = vec![0; stream+stream_bytes+stream_bytes.div_ceil(1024)];
        blob[..4].copy_from_slice(b"DLT5");
        blob[4..8].copy_from_slice(&16000u32.to_le_bytes());
        blob[8..12].copy_from_slice(&frames.to_le_bytes());
        blob[12..16].copy_from_slice(&(stream_bytes as u32).to_le_bytes());
        blob[16] = 8; blob[18..20].copy_from_slice(&books.to_le_bytes()); blob[20] = 3; blob[21] = 2;
        for book in 0..books as usize { for channel in 0..2 { for code in 0..8 {
            let delta = DELTAS[(code+track*3+channel*5+book*7) % 8];
            let at = 32+book*32+channel*16+code*2;
            blob[at..at+2].copy_from_slice(&delta.to_le_bytes());
        } } }
        for frame in (0..frames).step_by(4) {
            let mut word = 0u32;
            for offset in 0..4 { word |= (source_code(track, frame+offset) as u32) << (offset*6); }
            let at = stream+frame as usize/4*3;
            blob[at..at+3].copy_from_slice(&word.to_le_bytes()[..3]);
        }
        let mut crc = 0u8;
        for index in 0..stream_bytes {
            crc ^= blob[stream+index];
            for _ in 0..8 { crc = (crc << 1) ^ if crc & 128 != 0 { 7 } else { 0 }; }
            if index & 1023 == 1023 || index+1 == stream_bytes { blob[stream+stream_bytes+index/1024] = crc; }
        }
        let pcm = decode_prefix(&blob, frames);
        platform::load(ADDRESSES[track], &blob);
        let mut descriptor = audio::AudioTrack::EMPTY;
        assert_eq!(audio::audio_track_read(&mut descriptor, ADDRESSES[track], blob.len() as u32, track as u8), 1);
        assert!(descriptor.frames == frames && descriptor.books == books);
        result.push(Asset { track: descriptor, pcm, bytes: blob.len() as u32 });
    }
    result
}

unsafe fn pump() {
    audio::audio_service(0); assert_eq!(value!(havoc_fault), 0);
    for _ in 0..400 { audio::havoc_raster_work(); }
    audio::audio_service(0); assert_eq!(value!(havoc_fault), 0);
}
unsafe fn begin(asset: &Asset) {
    platform::clear_ram(); assign!(havoc_fault, 0); assign!(audio_running, 0);
    assign!(havoc_underruns, 0); assign!(havoc_minimum_lead, 32767);
    audio::audio_init(&asset.track);
    let mut guard = 1000;
    while value!(produced) < audio::PRIME_FRAMES && guard != 0 { pump(); guard -= 1; }
    assert!(guard != 0 && value!(produced) < audio::LEAD_FRAMES);
    audio::audio_start(390);
}
/// Independent fade oracle from the published audible-song statics: full
/// volume until the final 16384 frames, then one step per 512 frames left.
unsafe fn expected_fade() -> u8 {
    let elapsed = value!(consumed).wrapping_sub(value!(audio_audible_start));
    let frames = value!(audio_audible_frames);
    let remaining = if elapsed < frames { frames-elapsed } else { 0 };
    if remaining >= 16384 { 32 } else { (remaining >> 9) as u8 }
}
unsafe fn consume(asset: &Asset, offset: u32, frames: u16) {
    assert!(offset+frames as u32 <= asset.track.frames);
    assert!(value!(produced).wrapping_sub(value!(consumed)) > frames as u32+256);
    for frame in 0..frames as u32 { for channel in 0..2u32 {
        let actual = platform::peek_reu(platform::RING_ADDRESS+(((value!(consumed)+frame)*2+channel) & 65535));
        let expected = asset.pcm[((offset+frame)*2+channel) as usize];
        assert_eq!(actual, expected, "track{} source{} channel{}", asset.track.id, offset+frame, channel);
    } }
    platform::TICKS = platform::TICKS.wrapping_sub(frames as u32*64);
    audio::audio_clock(); assert_eq!(value!(havoc_fault), 0);
    assert_eq!(value!(audio_fade), expected_fade(), "fade level at consumed {}", value!(consumed));
}
unsafe fn consume_to(asset: &Asset, mut offset: u32, end: u32) {
    while offset < end { let n = (end-offset).min(128) as u16; pump(); consume(asset, offset, n); offset += n as u32; }
}

#[test]
fn native_production_audio_exact_pcm_and_transport() { unsafe {
    let tracks = assets();
    // Packed output can be deliberately unaligned even on the host.
    let mut storage = [0u8; 21];
    let unaligned = storage.as_mut_ptr().add(1).cast::<audio::AudioTrack>();
    assert_eq!(audio::audio_track_read(unaligned, ADDRESSES[0], tracks[0].bytes, 0), 1);
    assert!(read_unaligned(unaligned).frames == FRAME_COUNTS[0]);
    assert_eq!(audio::audio_track_read(unaligned, ADDRESSES[0], tracks[0].bytes-1, 0), 0);
    platform::poke_reu(ADDRESSES[0]+21, 1);
    assert_eq!(audio::audio_track_read(unaligned, ADDRESSES[0], tracks[0].bytes, 0), 0);
    platform::poke_reu(ADDRESSES[0]+21, 2);

    begin(&tracks[0]);
    let stashes = platform::STASHES;
    pump(); pump(); let after = platform::STASHES; assert_eq!(after, stashes);
    audio::audio_queue(&tracks[1].track, 0);
    // A natural ending fades over its final 16384 frames and the incoming song
    // starts at full volume on the frame that crosses the boundary.
    let end = FRAME_COUNTS[0];
    consume_to(&tracks[0], 0, end-16512);
    assert_eq!((audio::test_boundary().0, value!(audio_fade)), (0, 32), "no boundary a second before the end");
    consume_to(&tracks[0], end-16512, end-8192); assert_eq!(value!(audio_fade), 16);
    consume_to(&tracks[0], end-8192, end-512); assert_eq!(value!(audio_fade), 1);
    assert_eq!(audio::test_boundary(), (1, end), "the decoder reaches the end within the lead");
    consume_to(&tracks[0], end-512, end-1); assert_eq!(value!(audio_fade), 0);
    assert_eq!(value!(audio_event), 0);
    consume(&tracks[0], end-1, 1);
    assert_eq!(value!(audio_event), 1); assert_eq!(value!(audio_audible_track), 1);
    assert_eq!(value!(audio_fade), 32);
    assert_eq!(audio::audio_remaining(), FRAME_COUNTS[1]); assign!(audio_event, 0);
    consume_to(&tracks[1], 0, FRAME_COUNTS[1]);
    assert_eq!(value!(audio_event), 1); assign!(audio_event, 0);
    consume_to(&tracks[1], 0, 32768);
    assert_eq!(value!(havoc_underruns), 0);

    // A cut is prepared while old chunks still pin book0 of the old track.
    platform::clear_ram(); assign!(havoc_fault, 0); assign!(audio_running, 0);
    audio::audio_init(&tracks[0].track); audio::audio_service(0);
    let cut = (0..6).map(|slot| array(addr_of!(audio::havoc_audio_chunk_units).cast(), slot) as u32*2).sum::<u32>();
    for _ in 0..66 { audio::havoc_raster_work(); }
    audio::audio_end_in(0); audio::audio_queue(&tracks[1].track, 0); audio::audio_service(0);
    while value!(produced) < audio::PRIME_FRAMES { pump(); }
    audio::audio_start(390);
    consume_to(&tracks[0], 0, cut);
    assert_eq!(value!(audio_event), 1); assert_eq!(value!(audio_audible_track), 1);
    assign!(audio_event, 0); consume_to(&tracks[1], 0, 32768);

    // A manual skip ends the audible song one fade ahead: the fade starts at
    // once, the queued cut waits for that end, and silence meets the switch.
    begin(&tracks[0]);
    audio::audio_queue(&tracks[1].track, 1); assert_eq!(audio::audio_remaining(), 16384);
    consume_to(&tracks[0], 0, 8192); assert_eq!(value!(audio_fade), 16);
    assert_eq!(audio::test_boundary().0, 0, "the cut must wait for the virtual end");
    consume_to(&tracks[0], 8192, 15872); assert_eq!(value!(audio_fade), 1);
    let (pending, boundary) = audio::test_boundary();
    assert!(pending != 0 && boundary >= 16384 && boundary < 16384+128, "cut boundary {boundary}");
    consume_to(&tracks[0], 15872, boundary-1); assert_eq!((value!(audio_fade), value!(audio_event)), (0, 0));
    consume(&tracks[0], boundary-1, 1);
    assert_eq!((value!(audio_event), value!(audio_audible_track), value!(audio_fade)), (1, 1, 32));
    assign!(audio_event, 0); consume_to(&tracks[1], 0, 4096);
    assert_eq!(value!(havoc_underruns), 0);
    // Ending later than the natural end never extends a song.
    begin(&tracks[0]); audio::audio_end_in(FRAME_COUNTS[0]+1); assert_eq!(audio::audio_remaining(), FRAME_COUNTS[0]);

    // Rebase both an ordinary timeline and a pending next-track boundary.
    for pending in [false, true] {
        let source = if pending { 0 } else { 2 };
        begin(&tracks[source]);
        if pending { audio::audio_queue(&tracks[1].track, 0); }
        consume_to(&tracks[source], 0, 32760);
        let remaining = audio::audio_remaining();
        audio::test_rebase_shift(0x00ff8000);
        assert_eq!(value!(consumed), 0x00fffff8);
        pump(); consume(&tracks[source], 32760, 8);
        assert_eq!(value!(consumed), 0); assert_eq!(audio::audio_remaining(), remaining-8);
        consume_to(&tracks[source], 32768, FRAME_COUNTS[source]);
        assert_eq!(value!(audio_event), 1);
        assert_eq!(value!(audio_audible_track), if pending { 1 } else { 2 });
    }

    begin(&tracks[0]);
    let operations = platform::TRANSFERS+platform::STASHES;
    audio::audio_service(platform::TICKS.wrapping_sub(10));
    assert_eq!(platform::TRANSFERS+platform::STASHES, operations);
    platform::TICKS = platform::TICKS.wrapping_sub(65536); audio::audio_clock();
    assert_eq!(value!(havoc_fault), 6); assert_eq!(value!(audio_running), 0);
    begin(&tracks[0]); assign!(produced, value!(consumed)+256);
    platform::TICKS = platform::TICKS.wrapping_sub(64); audio::audio_clock();
    assert_eq!(value!(havoc_fault), 5); assert_eq!(value!(havoc_underruns), 1);
    assert_eq!(value!(audio_running), 0);

    // Optional authenticated production file supplements the adversarial cases.
    // Set this explicitly in the production validation wrapper, never silently
    // substitute a diagnostic clip for a missing full song.
    if let Some(path) = std::env::var_os("HAVOC_AUDIO_TEST_DLT") {
        let blob = std::fs::read(&path).expect("read requested DLT5 production fixture");
        platform::load(0x400000, &blob);
        let mut track = audio::AudioTrack::EMPTY;
        assert_eq!(audio::audio_track_read(&mut track, 0x400000, blob.len() as u32, 1), 1);
        let asset = Asset { track, pcm: decode_prefix(&blob, 80000), bytes: blob.len() as u32 };
        begin(&asset); consume_to(&asset, 0, 80000);
        println!("Authenticated DLT5 input {:?}: 160000 output PCM bytes matched", path);
    }
} }

#[test]
fn bounded_division_matches_independent_native_arithmetic() {
    let divisors = [1800000u32, 2094545, 2097152, 2400000];
    let intervals = [0u32, 1, 32, 65, 4096, 65535];
    let mut cases = 0;
    for d in divisors { for elapsed in intervals { for carry in [0, 1, d/2, d-1] {
        let value = elapsed*32768+carry;
        let mut remainder = value;
        assert_eq!(audio::divide_bounded(&mut remainder, d << 10, 11) as u32, value/d);
        assert_eq!(remainder, value % d); cases += 1;
    } } }
    let mut seed = 0x640016u32;
    for index in 0..4096 {
        let d = divisors[index & 3];
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let value = (seed & 65535)*32768+seed % d;
        let mut remainder = value;
        assert_eq!(audio::divide_bounded(&mut remainder, d << 10, 11) as u32, value/d);
        assert_eq!(remainder, value % d); cases += 1;
    }
    for d in [1u32, 2, 7, 128, 65535] { for n in [0, d-1, d, d*255, d*256-1, d*256, u32::MAX] {
        let mut remainder = n;
        let q = (n/d).min(255);
        assert_eq!(audio::divide_bounded(&mut remainder, d << 7, 8) as u32, q);
        assert_eq!(remainder, n-q*d); cases += 1;
    } }
    assert_eq!(cases, 4227);
}

#[test]
fn audio_track_assembly_layout_is_byte_exact() {
    assert_eq!(core::mem::size_of::<audio::AudioTrack>(), 19);
    assert_eq!(core::mem::align_of::<audio::AudioTrack>(), 1);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, address), 0);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, frames), 4);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, stream), 8);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, books), 12);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, short_segment), 14);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, long_segments), 16);
    assert_eq!(core::mem::offset_of!(audio::AudioTrack, id), 18);
}
