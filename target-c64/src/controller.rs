//! Foreground ownership and playlist control. Timed drawing stays in assembly.
use core::{mem::MaybeUninit, ptr::{addr_of_mut, read_volatile, write_volatile}};
use crate::{audio::*, assets::*, musicdisk::*, panorama::*, platform::*};

extern "C" {
    pub static mut havoc_raster_x: u8;
    pub static mut havoc_raster_y: u8;
    pub static mut havoc_raster_top: u8;
    pub static mut havoc_raster_late_lines: u8;
    fn havoc_raster_init();
    fn havoc_raster_frame();
    fn havoc_raster_work();
}
#[used] #[no_mangle] pub static mut havoc_late_frames: u16 = 0;
#[used] #[no_mangle] pub static mut havoc_late_lines_total: u16 = 0;
static mut pending_track: MaybeUninit<AudioTrack> = MaybeUninit::zeroed();
static mut active_slot: u8 = 0;
static mut old_slot: u8 = 255;
static mut pending_slot: u8 = 255;
static mut requested: u8 = 0;
static mut descriptor_ready: u8 = 0;
static mut prepared: u8 = 0;
static mut manual_cut: u8 = 0;
static mut automatic: u8 = 1;
static mut muted: u8 = 0;
static mut applied_volume: u8 = 32; // sampler_arm() starts at full volume
static mut old_keys: u8 = 255;
static mut old_space: u8 = 255;
static mut frame_divider: u8 = 0;
static mut dx: i8 = 1;
static mut dy: i8 = 1;
/// The next picture's seed loads during the final quarter second of the
/// outgoing song, so the camera keeps scrolling through most of the fade.
const PREFETCH_FRAMES: u32 = crate::platform::SAMPLE_RATE / 4;

#[inline(always)]
unsafe fn pending() -> *mut AudioTrack { addr_of_mut!(pending_track).cast() }
#[cfg(not(test))] extern "C" { fn controller_adjacent(id: u8, delta: i8) -> u8; }
#[inline(always)]
pub fn adjacent(id: u8, delta: i8) -> u8 {
    #[cfg(test)] { adjacent_impl(id, delta) }
    #[cfg(not(test))] { unsafe { controller_adjacent(id, delta) } }
}
#[inline(never)]
#[cfg_attr(not(test), link_section = ".bankcode")]
#[cfg_attr(not(test), export_name = "controller_adjacent_banked")]
extern "C" fn adjacent_impl(id: u8, delta: i8) -> u8 {
    const _: () = assert!(TRACK_COUNT as usize == PLAYLIST.len());
    let mut i = 0u8;
    // The short-circuit bound and the wrap below keep both accesses in range.
    // Avoid a bounds-panic path into device I/O while the calculation is banked.
    while i < TRACK_COUNT && unsafe { *PLAYLIST.get_unchecked(i as usize) } != id { i += 1; }
    if i == TRACK_COUNT { i = 0; }
    if delta > 0 { i += 1; if i == TRACK_COUNT { i = 0; } }
    else { i = if i != 0 { i - 1 } else { TRACK_COUNT - 1 }; }
    unsafe { *PLAYLIST.get_unchecked(i as usize) }
}
unsafe fn keyrow(mask: u8) -> u8 { write(0xdc02, 255); write(0xdc03, 0); write(0xdc00, mask); read(0xdc01) }
pub unsafe fn camera_near_edge() -> bool {
    pano_camera_x.wrapping_sub(80) > pano_limit_x().wrapping_sub(160)
        || pano_camera_y.wrapping_sub(80) > pano_limit_y().wrapping_sub(160)
}
pub unsafe fn camera_step() {
    if pano_camera_x == 0 { dx = 1; } else if pano_camera_x >= pano_limit_x() { dx = -1; }
    if pano_camera_y == 0 { dy = 1; } else if pano_camera_y >= pano_limit_y() { dy = -1; }
    pano_request(dx, dy);
}
pub unsafe fn controls() {
    let keys = keyrow(254); let pressed = old_keys & !keys;
    old_keys = keys;
    let space = keyrow(127); let space_pressed = old_space & !space & 16;
    old_space = space;
    if prepared == 0 && audio_next_valid == 0 && pano_busy() == 0 && (pressed & 48 != 0 || space_pressed != 0) {
        requested = adjacent(audio_audible_track, if pressed & 32 != 0 || space_pressed != 0 { 1 } else { -1 });
        manual_cut = 1; descriptor_ready = 0;
    }
    if pressed & 64 != 0 { automatic = 1; }
    if pressed & 8 != 0 { muted ^= 1; }
    write(0xdc02, 0); write(0xdc00, 255); let joystick = read(0xdc00);
    if joystick & 15 != 15 {
        automatic = 0;
        if joystick & 1 == 0 { pano_request(0, -1); }
        if joystick & 2 == 0 { pano_request(0, 1); }
        if joystick & 4 == 0 { pano_request(-1, 0); }
        if joystick & 8 == 0 { pano_request(1, 0); }
    }
    if joystick & 16 == 0 { automatic = 1; }
    if automatic != 0 {
        frame_divider = frame_divider.wrapping_add(1);
        if frame_divider & 1 == 0 && pano_busy() == 0 { camera_step(); }
    }
    // Mute overrides the song-change fade; the sampler is written only on change.
    let level = if muted != 0 { 0 } else { audio_fade };
    if level != applied_volume { applied_volume = level; sampler_volume(level); }
}
/// Starts loading the picture of the loaded track `id`; a no-op while a
/// picture change is in progress.
unsafe fn picture(id: u8) {
    let bundle = disk_ready(id);
    if !bundle.is_null() { pano_prefetch((*bundle).panorama_reu, (*bundle).width, (*bundle).height); }
}
pub unsafe fn selection_step(deadline: u32) {
    if audio_event != 0 {
        audio_event = 0;
        // A song that restarted before its replacement could be queued (the load
        // landed inside the decoder lead) is cut after a fade instead of replayed.
        manual_cut |= prepared;
        if prepared != 0 && audio_audible_track == requested {
            old_slot = active_slot; active_slot = pending_slot; pending_slot = 255;
            prepared = 0; descriptor_ready = 0; manual_cut = 0;
            requested = adjacent(audio_audible_track, 1);
            // Normally a no-op: the picture began loading before the switch.
            picture(audio_audible_track);
        }
    }
    if old_slot != 255 {
        // The slide starts once the seed is in. The old slot is freed when the
        // picture change retires, or at once when no picture could load.
        if pano_can_transition() != 0 { pano_transition(); }
        if pano_busy() != 0 { return; }
        disk_release(old_slot); old_slot = 255;
    }
    if prepared != 0 {
        if pano_busy() == 0 && audio_remaining() < PREFETCH_FRAMES { picture(requested); }
        return;
    }
    let bundle = disk_ready(requested);
    if bundle.is_null() {
        if disk_busy == 0 && timer_read().wrapping_sub(deadline) as i32 > 400 { disk_request(requested); }
        return;
    }
    if descriptor_ready == 0 {
        if (timer_read().wrapping_sub(deadline) as i32) < 350 { return; }
        if audio_track_read(pending(), (*bundle).audio_reu, (*bundle).audio_bytes, (*bundle).track) == 0 {
            disk_error = DISK_HEADER; return;
        }
        descriptor_ready = 1;
    }
    // Pinned here, queued by run(): at once after a key press, otherwise four
    // seconds before the natural end.
    if manual_cut != 0 || audio_remaining() < 4 * crate::platform::SAMPLE_RATE {
        disk_pin((*bundle).slot); pending_slot = (*bundle).slot; prepared = 1;
    }
}
#[inline(never)]
#[link_section = ".tailcode"]
pub unsafe fn fail(color: u8) -> ! { sampler_stop(); write(0xd020, color); loop { core::hint::spin_loop(); } }

#[inline(never)]
#[link_section = ".disk_init"]
#[no_mangle]
unsafe extern "C" fn boot_relocate() {
    extern "C" {
        static mut __high_start: u8; static mut __high_end: u8; static mut __high_load: u8;
        static mut __mid_start: u8; static mut __mid_end: u8; static mut __mid_load: u8;
        static mut __tail_start: u8; static mut __tail_end: u8; static mut __tail_load: u8;
        static mut __bank_start: u8; static mut __bank_end: u8; static mut __bank_load: u8;
    }
    // Volatile copies cannot be folded into a helper that has not relocated yet.
    macro_rules! relocate { ($start:ident,$end:ident,$load:ident) => {{
        let start = addr_of_mut!($start); let source = addr_of_mut!($load);
        let count = (addr_of_mut!($end) as usize).wrapping_sub(start as usize);
        let mut n = 0;
        while n < count { write_volatile(start.add(n), read_volatile(source.add(n))); n += 1; }
    }} }
    relocate!(__high_start, __high_end, __high_load);
    relocate!(__mid_start, __mid_end, __mid_load);
    relocate!(__tail_start, __tail_end, __tail_load);
    // The destination is ordinary RAM only while I/O is hidden. This startup
    // loop executes from the still-resident boot overlay and performs no I/O.
    write(1, 0x34);
    relocate!(__bank_start, __bank_end, __bank_load);
    write(1, 0x35);
}
#[inline(never)]
#[link_section = ".disk_init"]
#[no_mangle]
unsafe extern "C" fn boot_prepare() -> *const DiskBundle {
    if disk_init() == 0 { fail(2); }
    requested = PLAYLIST[0];
    if disk_request(requested) == 0 { fail(2); }
    let first = loop {
        let first = disk_ready(requested); if !first.is_null() { break first; }
        disk_step(); if disk_error != 0 && disk_busy == 0 { fail(2); }
    };
    if audio_track_read(pending(), (*first).audio_reu, (*first).audio_bytes, (*first).track) == 0 { fail(8); }
    let mut n = 0u16;
    while n < 256 { write(0xe600 + n, 0); n += 1; }
    n = 0;
    while n < 256 { reu_stash(ram_ptr(0xe600), RING_ADDRESS + ((n as u32) << 8), 256); n += 1; }
    if read(0xdf21) != 0x10 { fail(4); }
    let divider = sampler_calibrate();
    if divider == 0 || sampler_ticks_per_ring < 1800000 || sampler_ticks_per_ring > 2400000 { fail(8); }
    audio_init(pending()); active_slot = (*first).slot; disk_pin(active_slot);
    while read_volatile(core::ptr::addr_of!(produced)) < PRIME_FRAMES {
        audio_service(0); if havoc_fault != 0 { fail(8); }
        n = 0; while n < 384 { havoc_raster_work(); n += 1; }
    }
    sampler_prepare(divider);
    first
}
#[inline(always)]
pub unsafe fn run() -> ! {
    platform_init(); write(0xd020, 0); boot_relocate();
    let first = boot_prepare();
    havoc_raster_init();
    if pano_init((*first).panorama_reu, (*first).width, (*first).height) == 0 { fail(8); }
    requested = adjacent(audio_audible_track, 1); descriptor_ready = 0; disk_request(requested);
    let lead_line = if havoc_raster_top >= 8 { havoc_raster_top as u16 - 8 } else { 263 + havoc_raster_top as u16 - 8 };
    while raster_line() == lead_line {} while raster_line() != lead_line {}
    audio_start(0); let mut late = havoc_raster_late_lines;
    loop {
        let before = timer_read();
        havoc_raster_frame(); let deadline = timer_read().wrapping_sub(1430);
        havoc_late_lines_total = havoc_late_lines_total.wrapping_add(havoc_raster_late_lines.wrapping_sub(late) as u16);
        late = havoc_raster_late_lines; audio_clock();
        controls();
        pano_tick();
        if havoc_fault == 0 { audio_service(deadline); }
        if havoc_fault == 0 && timer_read().wrapping_sub(deadline) as i32 > 180 {
            selection_step(deadline);
            if prepared != 0 && audio_next_valid == 0 { audio_queue(pending(), manual_cut); }
        }
        pano_step(6, deadline);
        if timer_read().wrapping_sub(deadline) as i32 > 180 { disk_step(); }
        if before.wrapping_sub(timer_read()) > 26000 { havoc_late_frames = havoc_late_frames.wrapping_add(1); }
    }
}

// Standalone host models inspect ownership without exporting production ABI.
#[cfg(test)]
pub mod test_access {
    use super::*;
    #[derive(Clone, Copy, Debug)]
    pub struct State {
        pub active: u8, pub old: u8, pub pending_slot: u8, pub requested: u8,
        pub descriptor_ready: u8, pub prepared: u8, pub manual_cut: u8,
        pub dx: i8, pub dy: i8, pub automatic: u8, pub muted: u8, pub applied_volume: u8,
    }
    pub unsafe fn state() -> State {
        State { active: active_slot, old: old_slot, pending_slot,
            requested, descriptor_ready, prepared, manual_cut, dx, dy, automatic, muted, applied_volume }
    }
    pub unsafe fn reset(next: u8) {
        pending_track = MaybeUninit::zeroed(); active_slot = 0; old_slot = 255;
        pending_slot = 255; requested = next; descriptor_ready = 0;
        prepared = 0; manual_cut = 0; automatic = 1; muted = 0; applied_volume = 32;
        old_keys = 255; old_space = 255; frame_divider = 0; dx = 1; dy = 1;
    }
    pub unsafe fn clear_descriptor() { descriptor_ready = 0; }
    pub unsafe fn pending_ptr() -> *const AudioTrack { pending() }
}
