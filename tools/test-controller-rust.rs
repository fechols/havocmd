// Host execution of the production Rust controller and panorama streamer.
// rustc --test -O --edition=2021 tools/test-controller-rust.rs -o controller-tests
#![allow(dead_code, non_upper_case_globals, static_mut_refs)]
#[path = "../target-c64/src/controller.rs"] mod controller;
#[path = "../target-c64/src/panorama.rs"] mod native_panorama;

mod assets {
    pub const TRACK_COUNT: u8 = 21;
    pub const PLAYLIST: [u8; 21] = [1,3,4,5,6,7,8,9,17,10,11,12,13,14,15,16,18,19,20,2,0];
}
mod platform {
    pub const SAMPLE_RATE: u32 = 16000;
    pub const RING_ADDRESS: u32 = 0xfe0000;
    pub static mut RAM: [u8; 65536] = [0; 65536];
    pub static mut REU: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];
    pub static mut NOW: u32 = 100000;
    pub static mut KEYS: u8 = 255;
    pub static mut SPACE_ROW: u8 = 255;
    pub static mut JOYSTICK: u8 = 255;
    pub static mut sampler_ticks_per_ring: u32 = 2100000;
    pub unsafe fn ram_ptr(at: u16) -> *mut u8 { core::ptr::addr_of_mut!(RAM).cast::<u8>().add(at as usize) }
    pub unsafe fn read(at: u16) -> u8 {
        match at {
            0xdc01 => {
                assert_eq!((RAM[0xdc02], RAM[0xdc03]), (255, 0));
                match RAM[0xdc00] { 254 => KEYS, 127 => SPACE_ROW, _ => 255 }
            },
            0xdc00 => JOYSTICK, _ => ram_ptr(at).read()
        }
    }
    pub unsafe fn write(at: u16, value: u8) { ram_ptr(at).write(value); }
    pub unsafe fn timer_read() -> u32 { NOW }
    pub unsafe fn reu_fetch(at: u32, dst: *mut u8, n: u16) {
        let base = ram_ptr(0) as usize;
        assert!(at as usize + n as usize <= 2 * 1024 * 1024);
        assert!((dst as usize) >= base && dst as usize + n as usize <= base + 65536);
        core::ptr::copy_nonoverlapping(core::ptr::addr_of!(REU).cast::<u8>().add(at as usize), dst, n as usize);
        NOW = NOW.wrapping_sub(n as u32 + 12);
    }
    pub unsafe fn reu_stash(_: *const u8, _: u32, _: u16) { unreachable!(); }
    pub unsafe fn sampler_stop() {}
    pub unsafe fn sampler_volume(level: u8) { write(0xdf21, level); write(0xdf41, level); }
    pub unsafe fn sampler_prepare(_: u16) {}
    pub unsafe fn sampler_calibrate() -> u16 { 1 }
    pub unsafe fn platform_init() {}
    pub unsafe fn raster_line() -> u16 { 0 }
}
mod audio {
    #[repr(C, packed)] #[derive(Clone, Copy)]
    pub struct AudioTrack { pub address: u32, pub frames: u32, pub stream: u32,
        pub books: u16, pub short_segment: u16, pub long_segments: u16, pub id: u8 }
    pub const PRIME_FRAMES: u32 = 3968;
    pub static mut audio_event: u8 = 0;
    pub static mut audio_next_valid: u8 = 0;
    pub static mut audio_fade: u8 = 32;
    pub static mut audio_audible_track: u8 = 0;
    pub static mut havoc_fault: u8 = 0;
    pub static mut produced: u32 = 0;
    pub static mut REMAINING: u32 = 160000;
    pub static mut DESCRIPTOR_VALID: bool = true;
    pub static mut BOUNDARY_WAITING: bool = false;
    pub static mut READS: u32 = 0;
    pub static mut QUEUES: u32 = 0;
    pub static mut QUEUE_ID: u8 = 255;
    pub static mut QUEUE_CUT: u8 = 0;
    pub unsafe fn audio_track_read(track: *mut AudioTrack, address: u32, bytes: u32, id: u8) -> u8 {
        READS += 1; assert_eq!(address, 0x1000 + id as u32 * 65536); assert_eq!(bytes, 45064);
        if !DESCRIPTOR_VALID { return 0; }
        core::ptr::write_bytes(track, 0, 1); (*track).id = id; 1
    }
    pub unsafe fn audio_remaining() -> u32 { REMAINING }
    pub unsafe fn audio_queue(track: *const AudioTrack, cut: u8) {
        if audio_next_valid != 0 || BOUNDARY_WAITING { return; }
        QUEUES += 1; QUEUE_ID = (*track).id; QUEUE_CUT = cut; audio_next_valid = 1;
    }
    pub unsafe fn audio_init(_: *const AudioTrack) {}
    pub unsafe fn audio_service(_: u32) {}
    pub unsafe fn audio_start(_: u32) {}
    pub unsafe fn audio_clock() {}
}
mod musicdisk {
    #[repr(C, packed)] #[derive(Clone, Copy)]
    pub struct DiskBundle { pub audio_reu: u32, pub audio_bytes: u32, pub frames: u32,
        pub panorama_reu: u32, pub panorama_bytes: u32, pub width: u16, pub height: u16,
        pub storage_width: u16, pub storage_height: u16, pub track: u8, pub slot: u8 }
    const EMPTY: DiskBundle = DiskBundle { audio_reu: 0, audio_bytes: 0, frames: 0, panorama_reu: 0,
        panorama_bytes: 0, width: 0, height: 0, storage_width: 0, storage_height: 0, track: 0, slot: 0 };
    pub const DISK_HEADER: u8 = 3;
    pub static mut disk_error: u8 = 0;
    pub static mut disk_busy: u8 = 0;
    pub static mut BUNDLES: [DiskBundle; 2] = [EMPTY; 2];
    pub static mut AVAILABLE: [bool; 2] = [false; 2];
    pub static mut PINS: [u8; 2] = [0; 2];
    pub static mut LOADING_SLOT: usize = 0;
    pub static mut LOADING_ID: u8 = 0;
    pub static mut REQUESTS: u32 = 0;
    pub static mut RELEASES: u32 = 0;
    pub unsafe fn set_bundle(slot: usize, id: u8) {
        BUNDLES[slot] = DiskBundle { slot: slot as u8, track: id, audio_reu: 0x1000 + id as u32 * 65536,
            audio_bytes: 45064, panorama_reu: 0x800000 + id as u32 * 65536, width: 905, height: 1086, ..EMPTY };
        AVAILABLE[slot] = true;
    }
    pub unsafe fn disk_ready(id: u8) -> *const DiskBundle {
        for i in 0..2 { if AVAILABLE[i] && BUNDLES[i].track == id {
            return core::ptr::addr_of!(BUNDLES).cast::<DiskBundle>().add(i);
        }}
        core::ptr::null()
    }
    pub unsafe fn disk_request(id: u8) -> u8 {
        REQUESTS += 1; if !disk_ready(id).is_null() { return 1; }
        if disk_busy != 0 { return 0; }
        LOADING_SLOT = if PINS[0] != 0 { 1 } else { 0 };
        if PINS[LOADING_SLOT] != 0 { return 0; }
        LOADING_ID = id; AVAILABLE[LOADING_SLOT] = false; disk_busy = 1; 1
    }
    pub unsafe fn complete_load() { set_bundle(LOADING_SLOT, LOADING_ID); disk_busy = 0; }
    pub unsafe fn disk_pin(slot: u8) { assert!(slot < 2); PINS[slot as usize] += 1; }
    pub unsafe fn disk_release(slot: u8) {
        assert!(slot < 2 && PINS[slot as usize] != 0); PINS[slot as usize] -= 1; RELEASES += 1;
    }
    pub unsafe fn disk_init() -> u8 { 1 }
    pub unsafe fn disk_step() {}
}
mod panorama {
    pub use crate::native_panorama::{pano_camera_x, pano_camera_y, pano_retired, pano_limit_x, pano_limit_y};
    pub static mut CAMERA_MODEL: bool = false;
    pub static mut BUSY: u8 = 0;
    pub static mut READY: u8 = 0;
    pub static mut PREFETCHES: u32 = 0;
    pub static mut TRANSITIONS: u32 = 0;
    pub static mut PREFETCH_ADDRESS: u32 = 0;
    pub static mut PICTURE_VALID: bool = true;
    pub unsafe fn pano_busy() -> u8 { if CAMERA_MODEL { crate::native_panorama::pano_busy() } else { BUSY } }
    pub unsafe fn pano_prefetch(address: u32, w: u16, h: u16) -> u8 {
        assert_eq!((w, h), (905, 1086)); if BUSY != 0 || !PICTURE_VALID { return 0; }
        BUSY = 1; READY = 0; pano_retired = 0; PREFETCH_ADDRESS = address; PREFETCHES += 1; 1
    }
    // Ready only until the slide starts, as in the real panorama.
    pub unsafe fn pano_can_transition() -> u8 { READY }
    pub unsafe fn pano_transition() { assert!(READY != 0 && BUSY != 0); READY = 0; TRANSITIONS += 1; }
    pub unsafe fn pano_transition_done() -> u8 { pano_retired }
    pub unsafe fn pano_request(dx: i8, dy: i8) { if CAMERA_MODEL { crate::native_panorama::pano_request(dx, dy); } }
    pub unsafe fn pano_tick() { crate::native_panorama::pano_tick(); }
    pub unsafe fn pano_step(jobs: u8, deadline: u32) -> u8 { crate::native_panorama::pano_step(jobs, deadline) }
    pub unsafe fn pano_init(address: u32, w: u16, h: u16) -> u8 { crate::native_panorama::pano_init(address, w, h) }
}

#[no_mangle] pub static mut havoc_raster_x: u8 = 0;
#[no_mangle] pub static mut havoc_raster_y: u8 = 0;
#[no_mangle] pub static mut havoc_raster_top: u8 = 31;
#[no_mangle] pub static mut havoc_raster_late_lines: u8 = 0;
#[no_mangle] pub static mut havoc_tilt_k: i8 = 0;
#[no_mangle] pub unsafe extern "C" fn havoc_raster_init() {}
#[no_mangle] pub unsafe extern "C" fn havoc_raster_frame() {}
#[no_mangle] pub unsafe extern "C" fn havoc_raster_work() {}
// Relocation is unreachable in these host tests, but its linker symbols retain
// definitions so even host linkers that collect whole custom sections work.
#[no_mangle] static mut __high_start: u8 = 0;
#[no_mangle] static mut __high_end: u8 = 0;
#[no_mangle] static mut __high_load: u8 = 0;
#[no_mangle] static mut __mid_start: u8 = 0;
#[no_mangle] static mut __mid_end: u8 = 0;
#[no_mangle] static mut __mid_load: u8 = 0;
#[no_mangle] static mut __tail_start: u8 = 0;
#[no_mangle] static mut __tail_end: u8 = 0;
#[no_mangle] static mut __tail_load: u8 = 0;
#[no_mangle] static mut __bank_start: u8 = 0;
#[no_mangle] static mut __bank_end: u8 = 0;
#[no_mangle] static mut __bank_load: u8 = 0;

#[cfg(test)] mod tests {
    use super::*;
    use controller::test_access as access;
    unsafe fn frame() {
        controller::selection_step(platform::NOW.wrapping_sub(1000));
        let s = access::state();
        if s.prepared != 0 && audio::audio_next_valid == 0 {
            audio::audio_queue(access::pending_ptr(), s.manual_cut);
        }
    }
    unsafe fn selection() {
        use audio::*; use musicdisk::*; use panorama::*;
        let first = assets::PLAYLIST[0]; let next = assets::PLAYLIST[1]; let previous = assets::PLAYLIST[20];
        access::reset(next); audio_audible_track = first; set_bundle(0, first); disk_pin(0);
        frame(); assert!(disk_busy != 0 && LOADING_ID == next);
        let saved = REQUESTS; frame(); assert_eq!(REQUESTS, saved); complete_load();
        controller::selection_step(platform::NOW - 10); assert_eq!((access::state().descriptor_ready, READS), (0, 0));
        frame(); assert_eq!((access::state().descriptor_ready, access::state().prepared, (*access::pending_ptr()).id, READS), (1, 0, next, 1));
        // A key edge records a replacement but does not issue a loader command.
        let saved = REQUESTS; platform::KEYS = !16; controller::controls();
        let s = access::state(); assert_eq!((s.requested, s.manual_cut, s.descriptor_ready, disk_busy, REQUESTS), (previous, 1, 0, 0, saved));
        controller::selection_step(platform::NOW - 400); assert_eq!((disk_busy, REQUESTS), (0, saved));
        controller::selection_step(platform::NOW - 401); assert_eq!((disk_busy, REQUESTS), (1, saved + 1));
        frame(); assert_eq!((access::state().prepared, READS), (0, 1)); complete_load();
        // A skip pins the slot and queues the cut song as soon as its descriptor
        // is read; the picture waits until the song is a quarter second from its end.
        frame(); assert_eq!((access::state().descriptor_ready, access::state().prepared, READS, PINS[1]), (1, 1, 2, 1));
        assert_eq!((QUEUES, QUEUE_ID, QUEUE_CUT, PREFETCHES), (1, previous, 1, 0));
        frame(); assert_eq!((access::state().prepared, (*access::pending_ptr()).id, READS, PINS[1], PREFETCHES), (1, previous, 2, 1, 0));
        // Further previous/next input is ignored while the prepared bundle owns its slot.
        platform::KEYS = !32; controller::controls(); assert_eq!(access::state().requested, previous);
        audio_next_valid = 0; BOUNDARY_WAITING = true; frame(); frame();
        assert_eq!((QUEUES, access::state().prepared, access::state().active, PINS, PREFETCHES), (1, 1, 0, [1, 1], 0));
        REMAINING = 4000; frame(); assert_eq!(PREFETCHES, 0, "the camera scrolls until a quarter second remains");
        REMAINING = 3999; frame();
        let expected_art = BUNDLES[1].panorama_reu;
        assert_eq!((PREFETCH_ADDRESS, PREFETCHES, BUSY, TRANSITIONS), (expected_art, 1, 1, 0));
        frame(); assert_eq!((PREFETCHES, QUEUES), (1, 1), "one prefetch and one queue per song change");
        // The switch may land before the seed is in: the slide waits for it.
        BOUNDARY_WAITING = false; audio_audible_track = previous; audio_event = 1; frame();
        assert_eq!((access::state().prepared, access::state().active, access::state().old, TRANSITIONS, RELEASES, PREFETCHES), (0, 1, 0, 0, 0, 1));
        READY = 1; frame(); assert_eq!((TRANSITIONS, RELEASES), (1, 0));
        let saved = REQUESTS; frame(); assert_eq!((REQUESTS, PINS[0], TRANSITIONS), (saved, 1, 1));
        pano_retired = 1; BUSY = 0; READY = 0; REMAINING = 160000; frame();
        assert_eq!((access::state().old, RELEASES, PINS), (255, 1, [0, 1]));
        assert_eq!((access::state().requested, (*access::pending_ptr()).id, access::state().descriptor_ready), (first, first, 1));
        access::clear_descriptor(); DESCRIPTOR_VALID = false; REMAINING = 1; frame();
        assert_eq!((disk_error, access::state().prepared, PINS[0]), (DISK_HEADER, 0, 0));
        DESCRIPTOR_VALID = true; frame(); assert_eq!((access::state().prepared, PINS[0], (*access::pending_ptr()).id), (1, 1, first));
        // A natural ending inside the prefetch window loads its picture on the next step.
        frame(); assert_eq!((PREFETCHES, PREFETCH_ADDRESS, QUEUES, QUEUE_CUT), (2, BUNDLES[0].panorama_reu, 2, 0));
        // A switch that arrives before the prefetch window (a starved selection
        // step) loads its picture at the event; one that cannot load any picture
        // frees the old slot at once.
        audio_next_valid = 0; BUSY = 0; READY = 0; pano_retired = 1;
        access::reset(next); audio_audible_track = first; PINS = [1, 0]; set_bundle(1, next);
        REMAINING = 160000; frame(); assert_eq!((access::state().descriptor_ready, access::state().prepared), (1, 0));
        REMAINING = 63999; frame(); assert_eq!((access::state().prepared, PINS, QUEUES, QUEUE_CUT, PREFETCHES), (1, [1, 1], 3, 0, 2));
        audio_next_valid = 0; audio_audible_track = next; audio_event = 1; frame();
        assert_eq!((access::state().old, access::state().active, PREFETCHES, PREFETCH_ADDRESS, BUSY, RELEASES), (0, 1, 3, BUNDLES[1].panorama_reu, 1, 1));
        READY = 1; frame(); assert_eq!((TRANSITIONS, RELEASES), (2, 1));
        pano_retired = 1; BUSY = 0; READY = 0; frame(); assert_eq!((access::state().old, RELEASES, PINS), (255, 2, [0, 1]));
        audio_next_valid = 0; access::reset(next); audio_audible_track = first; PINS = [1, 0]; PICTURE_VALID = false;
        REMAINING = 63999; frame(); frame(); assert_eq!((access::state().prepared, PINS, QUEUES, PREFETCHES), (1, [1, 1], 4, 3));
        audio_next_valid = 0; audio_audible_track = next; audio_event = 1; frame();
        assert_eq!((access::state().old, RELEASES, PINS, TRANSITIONS, PREFETCHES, BUSY), (255, 3, [0, 1], 2, 3, 0));
        PICTURE_VALID = true; disk_busy = 0;
        // A restart that lands before the replacement could be queued (its load
        // finished inside the decoder lead) must not replay the whole song with a
        // frozen picture and dead keys: the deferred queue cuts after one fade.
        audio_next_valid = 0; BUSY = 0; READY = 0; pano_retired = 1;
        access::reset(next); audio_audible_track = first; PINS = [1, 0]; set_bundle(1, next);
        let (queued, prefetched) = (QUEUES, PREFETCHES);
        BOUNDARY_WAITING = true; REMAINING = 3999; frame(); frame();
        assert_eq!((access::state().prepared, access::state().manual_cut, QUEUES, PREFETCHES, BUSY), (1, 0, queued, prefetched + 1, 1));
        BOUNDARY_WAITING = false; audio_event = 1; REMAINING = 160000; frame();
        assert_eq!((access::state().prepared, access::state().manual_cut, access::state().old, QUEUES, QUEUE_ID, QUEUE_CUT),
            (1, 1, 255, queued + 1, next, 1), "a restart before the queue must cut the replay");
        audio_next_valid = 0; audio_audible_track = next; audio_event = 1; frame();
        assert_eq!((access::state().old, access::state().active, access::state().manual_cut), (0, 1, 0));
        READY = 1; frame(); pano_retired = 1; BUSY = 0; READY = 0; frame();
        assert_eq!((access::state().old, PINS), (255, [0, 1])); disk_busy = 0;
        // Fresh next-key test at the wrap boundary, including clock wrap admission.
        access::reset(first); CAMERA_MODEL = false; BUSY = 0; PINS = [0, 1]; AVAILABLE[0] = false;
        audio_next_valid = 0; audio_audible_track = previous; platform::KEYS = !32;
        controller::controls(); assert_eq!((access::state().requested, access::state().manual_cut), (first, 1));
        let saved = REQUESTS; platform::NOW = 17;
        controller::selection_step(platform::NOW.wrapping_sub(400)); assert_eq!(REQUESTS, saved);
        controller::selection_step(platform::NOW.wrapping_sub(401)); assert_eq!(REQUESTS, saved + 1);
    }
    unsafe fn spacebar_edges() {
        use audio::*; use panorama::*;
        let first=assets::PLAYLIST[0]; let next=assets::PLAYLIST[1]; let after=assets::PLAYLIST[2];
        access::reset(next); CAMERA_MODEL=false; BUSY=0;
        audio_next_valid=0; audio_audible_track=first;
        platform::KEYS=255; platform::JOYSTICK=255;
        // A different column on row7 must not act as SPACE.
        platform::SPACE_ROW=!32; controller::controls();
        assert_eq!(access::state().manual_cut,0);
        platform::SPACE_ROW=!16; controller::controls();
        assert_eq!((access::state().requested,access::state().manual_cut),(next,1));
        audio_audible_track=next;
        for _ in 0..4 {controller::controls();}
        assert_eq!(access::state().requested,next,"held SPACE repeated the skip");
        platform::SPACE_ROW=255; controller::controls();
        assert_eq!(access::state().requested,next,"release must not skip");
        platform::SPACE_ROW=!16; controller::controls();
        assert_eq!(access::state().requested,after,"fresh press must skip again");
        // A blocked press is consumed, not deferred until the queued song starts.
        access::reset(next); audio_audible_track=first; audio_next_valid=1;
        controller::controls(); assert_eq!(access::state().manual_cut,0);
        audio_next_valid=0; controller::controls();
        assert_eq!(access::state().manual_cut,0,"held blocked SPACE retriggered");
        platform::SPACE_ROW=255; controller::controls();
        platform::SPACE_ROW=!16; controller::controls();
        assert_eq!((access::state().requested,access::state().manual_cut),(next,1));
        // Reset the controller with SPACE still down: its startup edge state is clean.
        access::reset(next); controller::controls(); assert_eq!(access::state().manual_cut,1);
        platform::SPACE_ROW=255; platform::KEYS=!8; controller::controls();
        assert_eq!((access::state().muted,platform::RAM[0xdf21],platform::RAM[0xdf41]),(1,0,0));
        controller::controls(); assert_eq!(access::state().muted,1,"held F7 toggled again");
        // A song-change fade stays silent while muted, resumes at its own level on
        // unmute, and an unchanged level never rewrites the sampler registers.
        audio::audio_fade=20; controller::controls();
        assert_eq!((platform::RAM[0xdf21],platform::RAM[0xdf41]),(0,0),"mute must override the fade");
        platform::KEYS=255; controller::controls(); platform::KEYS=!8; controller::controls();
        assert_eq!((access::state().muted,platform::RAM[0xdf21],platform::RAM[0xdf41]),(0,20,20),"unmute must resume at the fade level");
        audio::audio_fade=32; platform::KEYS=255; controller::controls();
        assert_eq!((access::state().applied_volume,platform::RAM[0xdf21],platform::RAM[0xdf41]),(32,32,32));
        platform::RAM[0xdf21]=77; controller::controls();
        assert_eq!(platform::RAM[0xdf21],77,"unchanged level must not rewrite the sampler");
        platform::RAM[0xdf21]=32;
        platform::JOYSTICK=254; controller::controls();
        assert_eq!(access::state().automatic,0);
        platform::JOYSTICK=255; platform::KEYS=!64; controller::controls();
        assert_eq!(access::state().automatic,1,"F5 must still resume automatic camera");
        platform::KEYS=255; platform::SPACE_ROW=255;
        println!("SPACE row7/bit4: fresh/held/released/repressed, queued-input blocking, reset, F5/F7 isolation and mute/fade precedence passed.");
    }
    fn pixel(x: u32, y: u32) -> u8 {
        let x = x.min(904); let y = y.min(1085);
        ((x * 13) ^ (y * 7) ^ (x >> 4) ^ (x >> 8) ^ (y >> 6) ^ 22) as u8 & 15
    }
    unsafe fn art() {
        let mut at = 0x100000;
        for x in (0..256).step_by(2) { for y in 0..256 {
            platform::REU[at] = pixel(320 + x, 408 + y) | pixel(321 + x, 408 + y) << 4; at += 1;
        }}
        for tx in (0..912).step_by(16) { for ty in (0..1088).step_by(8) {
            for x in (0..16).step_by(2) { for y in 0..8 {
                platform::REU[at] = pixel(tx + x, ty + y) | pixel(tx + x + 1, ty + y) << 4; at += 1;
            }}
        }}
    }
    unsafe fn check_frame() {
        assert_eq!(havoc_tilt_k,0,"scrolling player must remain level");
        for row in 0..240u32 {
            for column in 0..200u32 {
                let px=(havoc_raster_x as u32+column)&255;
                let py=(havoc_raster_y as u32+row)&255;
                let packed=platform::RAM[0x4000+(px as usize/2)*256+py as usize];
                let got=(packed>>(4*(px&1)))&15;
                let expected=pixel(panorama::pano_camera_x as u32+column,panorama::pano_camera_y as u32+row);
                assert_eq!(got,expected,"camera=({},{}),row={row},column={column}",panorama::pano_camera_x,panorama::pano_camera_y);
            }
        }
    }
    unsafe fn camera() {
        use panorama::*;
        art(); access::reset(3); CAMERA_MODEL = true;
        assert_eq!(pano_init(0x100000, 905, 1086), 1);
        let mut edges=0; let mut resumed=false; let mut frames=0; let mut held_requests=0;
        for i in 0..20000 {
            let (before_x,before_y)=(pano_camera_x,pano_camera_y);
            if i&1==0 {controller::camera_step();}
            // Match the production order: publish ready movement before
            // spending this frame's remaining time on proactive cache work.
            pano_tick();
            if i&1==0 && (pano_camera_x,pano_camera_y)==(before_x,before_y) {held_requests+=1;}
            check_frame(); frames+=1;
            if pano_camera_x == 0 { edges |= 1; assert_eq!(havoc_tilt_k, 0); }
            if pano_camera_x == 705 { edges |= 2; assert_eq!(havoc_tilt_k, 0); }
            if pano_camera_y == 0 { edges |= 4; assert_eq!(havoc_tilt_k, 0); }
            if pano_camera_y == 846 { edges |= 8; assert_eq!(havoc_tilt_k, 0); }
            let s = access::state();
            if before_x == 0 && pano_camera_x > 0 { assert_eq!(s.dx, 1); }
            if before_x == 705 && pano_camera_x < 705 { assert_eq!(s.dx, -1); }
            if before_y == 0 && pano_camera_y > 0 { assert_eq!(s.dy, 1); }
            if before_y == 846 && pano_camera_y < 846 { assert_eq!(s.dy, -1); }
            if edges==15 && !controller::camera_near_edge() && (pano_camera_x,pano_camera_y)!=(before_x,before_y) {resumed=true;}
            pano_step(6,platform::NOW.wrapping_sub(50000));
            if edges==15 && resumed && frames>=6500 { break; }
        }
        assert_eq!(edges, 15); assert!(resumed);
        assert_eq!(held_requests,0,"rectangular prefetch must keep every requested camera step ready");
        println!("Controller/cache: {frames} exact200x240 original-art frames, zero held camera requests.");
        // Strictly outside [80,625]x[80,766] flattens; boundaries themselves do not.
        for (x, y, expected) in [(79,400,true),(80,400,false),(625,400,false),(626,400,true),
                                 (350,79,true),(350,80,false),(350,766,false),(350,767,true)] {
            pano_camera_x = x; pano_camera_y = y; assert_eq!(controller::camera_near_edge(), expected);
        }
    }
    #[test]
    fn selection_ownership_playlist_and_real_camera_edges() { unsafe {
        // Keep the mock asset constants tied to the generated production directory.
        let header = include_str!("../target-c64/include/musicdisk_assets.h");
        let line = header.lines().find(|s| s.contains("musicdisk_playlist[21]")).unwrap();
        let parsed: Vec<u8> = line.split('{').nth(1).unwrap().split('}').next().unwrap()
            .split(',').map(|v| v.trim().parse().unwrap()).collect();
        assert_eq!(parsed.as_slice(), assets::PLAYLIST);
        for (i, &id) in assets::PLAYLIST.iter().enumerate() {
            assert_eq!(controller::adjacent(id, 1), assets::PLAYLIST[(i + 1) % 21]);
            assert_eq!(controller::adjacent(id, -1), assets::PLAYLIST[(i + 20) % 21]);
        }
        // The banked implementation uses proven in-range unchecked reads.
        // Cover every byte ID, including the absent-ID fallback, and both
        // delta signs plus zero; the reference uses checked host indexing.
        for id in 0u16..=255 {
            for delta in [-128i8,-1,0,1,127] {
                let position=assets::PLAYLIST.iter().position(|&track|track==id as u8).unwrap_or(0);
                let next=if delta>0 {(position+1)%assets::PLAYLIST.len()}
                         else {(position+assets::PLAYLIST.len()-1)%assets::PLAYLIST.len()};
                assert_eq!(controller::adjacent(id as u8,delta),assets::PLAYLIST[next],"id={id},delta={delta}");
            }
        }
        selection(); spacebar_edges(); camera();
        println!("Rust controller:21-track order/wrap, manual prev/next400/401-tick guard and clock wrap, descriptor replacement/recovery, prepared pins, audible boundary and wipe retirement; real camera/cache reaches/reverses all edges level and resumes scrolling.");
    }}
}
