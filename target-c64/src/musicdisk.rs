//! Read-only Ultimate DOS loader with two pinned bundle slots.
//! A timed-out LOAD_REU keeps its slot until the final reply is acknowledged.
use core::ptr::{addr_of_mut, null};
use crate::assets::{MUSICDISK_DIRECTORY_CRC, FORMAT_VERSION, TRACK_COUNT,
                    ASSET_COUNT, SAMPLE_RATE};
use crate::platform::{ram_ptr, reu_fetch, reu_stash};
use crate::uci::*;

pub const DISK_SLOT_BYTES: u32 = 0x500000;
pub const DISK_SLOT0: u32 = 0x001000;
pub const DISK_SLOT1: u32 = 0x501000;
pub const DISK_NO_TRACK: u8 = 255;
pub const DISK_NO_UCI: u8 = 1;
pub const DISK_NOT_FOUND: u8 = 2;
pub const DISK_HEADER: u8 = 3;
pub const DISK_IO: u8 = 5;
pub const DISK_SHORT_READ: u8 = 6;
pub const DISK_TIMEOUT: u8 = 7;
const VOLUME_BYTES: u32 = 0x1000000;
const HEADER_BYTES: u16 = 4096;
const CHUNK_BYTES: u16 = 16384;
const OPEN: u8 = 1;
const SEEK: u8 = 2;
const LOAD: u8 = 3;
const CLOSE: u8 = 4;
const DONE: u8 = 5;
const FAILED: u8 = 6;

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct DiskBundle {
    pub audio_reu: u32, pub audio_bytes: u32, pub frames: u32,
    pub panorama_reu: u32, pub panorama_bytes: u32,
    pub width: u16, pub height: u16, pub storage_width: u16, pub storage_height: u16,
    pub track: u8, pub slot: u8,
}
const EMPTY: DiskBundle = DiskBundle { audio_reu: 0, audio_bytes: 0, frames: 0,
    panorama_reu: 0, panorama_bytes: 0, width: 0, height: 0,
    storage_width: 0, storage_height: 0, track: 0, slot: 0 };
const _: [(); 30] = [(); core::mem::size_of::<DiskBundle>()];

#[used] #[no_mangle] pub static mut disk_error: u8 = 0;
#[used] #[no_mangle] pub static mut disk_busy: u8 = 0;
#[used] #[no_mangle] pub static mut disk_chunks: u16 = 0;
static mut FOLDER: [u8; 96] = [0; 96];
static mut COMMAND: [u8; 112] = [0; 112];
static mut ENTRY: [u8; 64] = [0; 64];
static mut SLOTS: [DiskBundle; 2] = [EMPTY; 2];
static mut PINS: [u8; 2] = [0; 2];
static mut READY: [u8; 2] = [0; 2];
static mut SELECTED: u8 = 0;
static mut ASSET: u8 = 0;
static mut VOLUME: u8 = 0;
static mut STATE: u8 = 0;
static mut FILE_OPEN: u8 = 0;
static mut SOURCE: u32 = 0;
static mut REMAINING: u32 = 0;
static mut DESTINATION: u32 = 0;
static mut REQUESTED: u16 = 0;

#[inline(always)] unsafe fn folder() -> *mut u8 { addr_of_mut!(FOLDER).cast() }
#[inline(always)] unsafe fn command() -> *mut u8 { addr_of_mut!(COMMAND).cast() }
#[inline(always)] unsafe fn entry() -> *mut u8 { addr_of_mut!(ENTRY).cast() }
#[inline(always)] unsafe fn slot(n: u8) -> *mut DiskBundle { addr_of_mut!(SLOTS).cast::<DiskBundle>().add(n as usize) }
#[inline(always)] unsafe fn pin(n: u8) -> *mut u8 { addr_of_mut!(PINS).cast::<u8>().add(n as usize) }
#[inline(always)] unsafe fn ready(n: u8) -> *mut u8 { addr_of_mut!(READY).cast::<u8>().add(n as usize) }
#[inline(always)] unsafe fn header() -> *mut u8 { ram_ptr(0x4000) }
#[inline(always)] unsafe fn mounts() -> *mut u8 { ram_ptr(0x5000) }

#[inline(never)]
#[link_section = ".earlycode"]
unsafe fn get32(p: *const u8) -> u32 {
    *p as u32 | ((*p.add(1) as u32) << 8) |
        ((*p.add(2) as u32) << 16) | ((*p.add(3) as u32) << 24)
}
#[inline(never)]
#[link_section = ".earlycode"]
unsafe fn put32(p: *mut u8, mut value: u32) {
    let mut n = 0;
    while n < 4 { *p.add(n) = value as u8; value >>= 8; n += 1; }
}
#[inline(never)]
#[link_section = ".earlycode"]
unsafe fn copy_bytes(out: *mut u8, input: *const u8, n: u8) {
    let mut i = 0;
    while i < n { *out.add(i as usize) = *input.add(i as usize); i += 1; }
}
#[inline(never)]
unsafe fn fetch_entry(track: u8, panorama: bool) {
    let index = track + if panorama { TRACK_COUNT } else { 0 };
    reu_fetch(64 + (index as u32)*64, entry(), 64);
}
#[inline(never)]
#[link_section = ".earlycode"]
unsafe fn source_entry() {
    VOLUME = *entry().add(3); SOURCE = get32(entry().add(4)); REMAINING = get32(entry().add(8));
}
#[inline(never)]
#[link_section = ".earlycode"]
unsafe fn open_command(index: u8) -> u16 {
    let name = b"havocmd-1.reu";
    // set_folder permits at most95 bytes including its trailing slash, so
    // these offsets and the complete112-byte command fit in one byte.
    let mut i = 0u8; let mut n = 3u8;
    *command() = 1; *command().add(1) = 2; *command().add(2) = 1; // FA_READ
    while *folder().add(i as usize) != 0 {
        *command().add(n as usize) = *folder().add(i as usize); n += 1; i += 1;
    }
    i = 0;
    while i < name.len() as u8 {
        *command().add(n as usize) = if i == 8 { b'1'+index } else { *name.as_ptr().add(i as usize) };
        n += 1; i += 1;
    }
    n as u16
}
#[inline(never)]
unsafe fn start(opcode: u8, length: u16) -> u8 {
    *command() = 1; *command().add(1) = opcode;
    uci_start(command(), length, entry(), 64, 0)
}
#[inline(never)]
#[link_section = ".midcode"]
unsafe fn next_command() {
    if STATE == OPEN { let length = open_command(VOLUME); uci_start(command(), length, entry(), 64, 0); return; }
    if STATE == SEEK { put32(command().add(2), SOURCE); start(6, 6); return; }
    if STATE == LOAD {
        // Stop at64KiB and16MiB boundaries; no run-time wide division.
        let edge = 0u16.wrapping_sub(SOURCE as u16);
        REQUESTED = CHUNK_BYTES;
        if REMAINING < REQUESTED as u32 { REQUESTED = REMAINING as u16; }
        if edge != 0 && edge < REQUESTED { REQUESTED = edge; }
        put32(command().add(2), DESTINATION); put32(command().add(6), REQUESTED as u32);
        start(0x21, 10); return;
    }
    start(3, 2);
}

#[no_mangle]
#[inline(never)]
pub unsafe extern "C" fn disk_ready(track: u8) -> *const DiskBundle {
    let mut i = 0;
    while i < 2 { if *ready(i) != 0 && (*slot(i)).track == track { return slot(i); } i += 1; }
    null()
}
#[no_mangle]
#[inline(never)]
pub unsafe extern "C" fn disk_pin(n: u8) { if n < 2 && *pin(n) != 255 { *pin(n) += 1; } }
#[no_mangle]
#[inline(never)]
pub unsafe extern "C" fn disk_release(n: u8) { if n < 2 && *pin(n) != 0 { *pin(n) -= 1; } }

#[no_mangle]
#[inline(never)]
pub unsafe extern "C" fn disk_request(track: u8) -> u8 {
    if track >= TRACK_COUNT { return 0; }
    if !disk_ready(track).is_null() { return 1; }
    if disk_busy != 0 || uci_busy != 0 { return 0; }
    SELECTED = if *pin(0) != 0 { 1 } else { 0 };
    if *pin(SELECTED) != 0 { return 0; }
    let b = slot(SELECTED);
    *ready(SELECTED) = 0; (*b).track = track; (*b).slot = SELECTED;
    (*b).audio_reu = if SELECTED != 0 { DISK_SLOT1 } else { DISK_SLOT0 };
    fetch_entry(track, false);
    copy_bytes(addr_of_mut!((*b).audio_bytes).cast(), entry().add(8), 4);
    copy_bytes(addr_of_mut!((*b).frames).cast(), entry().add(20), 4);
    fetch_entry(track, true);
    copy_bytes(addr_of_mut!((*b).panorama_bytes).cast(), entry().add(8), 4);
    copy_bytes(addr_of_mut!((*b).width).cast(), entry().add(16), 2);
    copy_bytes(addr_of_mut!((*b).height).cast(), entry().add(20), 2);
    copy_bytes(addr_of_mut!((*b).storage_width).cast(), entry().add(24), 2);
    copy_bytes(addr_of_mut!((*b).storage_height).cast(), entry().add(28), 2);
    // Startup validated lengths and the sum; the directory never overlaps I/O.
    (*b).panorama_reu = (*b).audio_reu + (*b).audio_bytes;
    fetch_entry(track, false); source_entry(); DESTINATION = (*b).audio_reu;
    disk_error = 0; disk_busy = 1; FILE_OPEN = 0; ASSET = 0; STATE = OPEN;
    next_command(); 1
}

#[no_mangle]
#[inline(never)]
pub unsafe extern "C" fn disk_step() {
    if disk_busy == 0 { return; }
    uci_step();
    if uci_error != 0 { disk_error = if uci_error == UCI_TIMEOUT { DISK_TIMEOUT } else { DISK_IO }; }
    if uci_complete == 0 { return; } // Timeout never cancels an in-flight load.
    uci_complete = 0;
    if uci_status_ok() == 0 && disk_error == 0 { disk_error = DISK_IO; }
    if STATE == OPEN && uci_status_ok() != 0 { FILE_OPEN = 1; }
    if STATE == CLOSE { FILE_OPEN = 0; }
    if disk_error != 0 {
        if FILE_OPEN != 0 { STATE = CLOSE; next_command(); return; }
        STATE = FAILED; disk_busy = 0; return;
    }
    if STATE == OPEN { STATE = SEEK; }
    else if STATE == SEEK { STATE = LOAD; }
    else if STATE == LOAD {
        if uci_transfer_matches(REQUESTED as u32, DESTINATION) == 0 {
            disk_error = DISK_SHORT_READ; STATE = CLOSE; next_command(); return;
        }
        SOURCE += REQUESTED as u32; REMAINING -= REQUESTED as u32; DESTINATION += REQUESTED as u32;
        disk_chunks = disk_chunks.wrapping_add(1);
        if REMAINING == 0 || SOURCE == VOLUME_BYTES { STATE = CLOSE; }
    } else if STATE == CLOSE {
        if REMAINING != 0 { VOLUME += 1; SOURCE = HEADER_BYTES as u32; STATE = OPEN; }
        else if ASSET == 0 { ASSET = 1; fetch_entry((*slot(SELECTED)).track, true); source_entry(); STATE = OPEN; }
        else { *ready(SELECTED) = 1; STATE = DONE; disk_busy = 0; return; }
    }
    next_command();
}

// Startup overlay: every function below retires before the panorama cache.
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn transact(length: u16, out: *mut u8, capacity: u16) -> bool {
    if uci_start(command(), length, out, capacity, 0) == 0 { return false; }
    while uci_busy != 0 { uci_step(); if uci_error != 0 { return false; } }
    uci_error == 0
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn simple(opcode: u8) -> bool {
    *command() = 1; *command().add(1) = opcode;
    transact(2, entry(), 64) && uci_status_ok() != 0
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn crc32(data: *const u8, count: u16, is_header: bool) -> u32 {
    let mut crc = 0xffffffffu32; let mut i = 0;
    while i < count {
        let value = if is_header && i >= 32 && i < 36 { 0 } else { *data.add(i as usize) };
        crc ^= value as u32;
        let mut bit = 0;
        while bit < 8 { crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 }; bit += 1; }
        i += 1;
    }
    crc ^ 0xffffffff
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn check_header(index: u8, count: u8) -> bool {
    let h = header(); let magic = b"HVMDVOL";
    let mut i = 0usize;
    while i < 7 { if *h.add(i) != *magic.as_ptr().add(i) { return false; } i += 1; }
    if *h.add(7) != 0 || *h.add(8) != FORMAT_VERSION || *h.add(9) != 0 ||
        *h.add(10) != 0 || *h.add(11) != 16 || *h.add(12) != index || *h.add(13) != count ||
        count == 0 || count > 4 ||
        *h.add(14) != ASSET_COUNT || *h.add(15) != 0 || get32(h.add(16)) != VOLUME_BYTES ||
        get32(h.add(20)) < HEADER_BYTES as u32 || get32(h.add(20)) > VOLUME_BYTES ||
        get32(h.add(24)) != MUSICDISK_DIRECTORY_CRC || crc32(h.add(64), 4032, false) != MUSICDISK_DIRECTORY_CRC ||
        crc32(h, HEADER_BYTES, true) != get32(h.add(32)) { return false; }
    i = 36;
    while i < 64 { if *h.add(i) != 0 { return false; } i += 1; }
    i = 64 + ASSET_COUNT as usize*64;
    while i < HEADER_BYTES as usize { if *h.add(i) != 0 { return false; } i += 1; }
    i = 0;
    while i < ASSET_COUNT as usize {
        let p = h.add(64+i*64); let offset = get32(p.add(4)); let length = get32(p.add(8));
        if *p != (i % TRACK_COUNT as usize) as u8 || *p.add(1) != 0 ||
            *p.add(2) != if i < TRACK_COUNT as usize { 1 } else { 2 } ||
            *p.add(3) >= count || offset < HEADER_BYTES as u32 || offset >= VOLUME_BYTES || length == 0 { return false; }
        let capacity = VOLUME_BYTES-offset+(count-1-*p.add(3)) as u32*(VOLUME_BYTES-HEADER_BYTES as u32);
        if length > capacity { return false; }
        if i < TRACK_COUNT as usize {
            if get32(p.add(16)) != SAMPLE_RATE || get32(p.add(20)) == 0 ||
                get32(p.add(24)) != 2 || get32(p.add(28)) != 3 || length > DISK_SLOT_BYTES { return false; }
            if length+get32(p.add(TRACK_COUNT as usize*64+8)) > DISK_SLOT_BYTES { return false; }
        } else {
            let w = get32(p.add(16)); let height = get32(p.add(20));
            let sw = get32(p.add(24)); let sh = get32(p.add(28));
            if w < 256 || height < 256 || w > 65535 || height > 65535 || sw < w || sh < height ||
                sw > 65535 || sh > 65535 || sw & 15 != 0 || sh & 7 != 0 ||
                length != 32768 + sw*sh/2 { return false; }
        }
        i += 1;
    }
    true
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn set_folder(path: *const u8, mut length: u16) -> bool {
    while length != 0 && *path.add(length as usize-1) == 0 { length -= 1; }
    if length == 0 || length > 94 { return false; }
    let mut i = 0;
    while i < length { *folder().add(i as usize) = *path.add(i as usize); i += 1; }
    if *folder().add(length as usize-1) != b'/' { *folder().add(length as usize) = b'/'; length += 1; }
    *folder().add(length as usize) = 0; true
}
#[inline(never)]
#[link_section = ".disk_init"]
/// Volume 1 records how many 16 MiB volumes the release has (1..=4) and the
/// others must agree; only those files are opened.
unsafe fn try_folder() -> bool {
    let mut v = 0; let mut count = 1;
    while v < count {
        let length = open_command(v);
        if !transact(length, entry(), 64) || uci_status_ok() == 0 { return false; }
        let mut valid = simple(7) && uci_response_length >= 4 && get32(entry()) == VOLUME_BYTES;
        if valid {
            *command() = 1; *command().add(1) = 4; *command().add(2) = 0; *command().add(3) = 16;
            valid = transact(4, header(), HEADER_BYTES) && uci_response_length == HEADER_BYTES &&
                (uci_status_length == 0 || uci_status_ok() != 0) &&
                check_header(v, if v == 0 { *header().add(13) } else { count });
        }
        if !simple(3) { return false; }
        if !valid { disk_error = DISK_HEADER; return false; }
        if v == 0 { count = *header().add(13); reu_stash(header(), 0, HEADER_BYTES); }
        v += 1;
    }
    disk_error = 0; true
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn path_folder(opcode: u8) -> bool {
    *command() = 1; *command().add(1) = opcode;
    if !transact(2, header(), 256) || uci_status_ok() == 0 { return false; }
    set_folder(header(), uci_response_length) && try_folder()
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn fault() -> u8 {
    disk_error = if uci_error == UCI_ABSENT { DISK_NO_UCI }
        else if uci_error == UCI_TIMEOUT { DISK_TIMEOUT } else { DISK_IO };
    0
}

#[no_mangle]
#[inline(never)]
#[link_section = ".disk_init"]
pub unsafe extern "C" fn disk_init() -> u8 {
    let mut count = 0u8; let mut i = 0u8;
    disk_error = DISK_NOT_FOUND; disk_busy = 0; disk_chunks = 0;
    while i < 2 { *pin(i) = 0; *ready(i) = 0; (*slot(i)).track = DISK_NO_TRACK; i += 1; }
    if path_folder(0x12) { return 1; } // Current DOS path.
    if uci_error != 0 { return fault(); }
    if path_folder(0x17) { return 1; } // Configured Home; read-only path change.
    if uci_error != 0 { return fault(); }
    *command() = 1; *command().add(1) = 0x11; *command().add(2) = b'/';
    if !transact(3, entry(), 64) || uci_status_ok() == 0 || !simple(0x13) { return fault(); }
    *command() = 1; *command().add(1) = 0x14;
    if uci_start(command(), 2, entry(), 64, 1) == 0 { return fault(); }
    while uci_busy != 0 {
        uci_step(); if uci_error != 0 { return fault(); }
        if uci_packet_ready != 0 {
            let n = uci_response_length;
            if n > 1 && n <= 64 && *entry() & 16 != 0 && count < 32 && *entry().add(1) != b'.' {
                let name = mounts().add(count as usize*64); let mut j = 1;
                while j < n { *name.add(j as usize-1) = *entry().add(j as usize); j += 1; }
                *name.add(n as usize-1) = 0; count += 1;
            }
            uci_accept_packet();
        }
    }
    if uci_status_ok() == 0 { return fault(); }
    i = 0;
    while i < count {
        let name = mounts().add(i as usize*64); let suffix = b"/havocmd/";
        *folder() = b'/'; let mut j = 0usize;
        while *name.add(j) != 0 { *folder().add(j+1) = *name.add(j); j += 1; }
        j += 1; let mut n = 0;
        while n < suffix.len() { *folder().add(j) = *suffix.as_ptr().add(n); j += 1; n += 1; }
        *folder().add(j) = 0;
        if try_folder() { return 1; }
        if uci_error != 0 { return fault(); }
        i += 1;
    }
    0
}

#[cfg(test)]
pub unsafe fn test_check_header(index: u8, count: u8) -> bool { check_header(index, count) }
