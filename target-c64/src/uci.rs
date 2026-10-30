//! Bounded, single-flight Ultimate Command Interface client.
//!
//! Command/response storage must remain alive until `uci_busy` clears. A
//! timeout records an error; it never cancels firmware DMA or releases memory.
use core::ptr::{addr_of_mut, null, null_mut};
use crate::platform::{read, write, timer_read};

pub const UCI_ABSENT: u8 = 1;
pub const UCI_PROTOCOL: u8 = 2;
pub const UCI_OVERFLOW: u8 = 3;
pub const UCI_TIMEOUT: u8 = 4;

#[used] #[no_mangle] pub static mut uci_error: u8 = 0;
#[used] #[no_mangle] pub static mut uci_busy: u8 = 0;
#[used] #[no_mangle] pub static mut uci_complete: u8 = 0;
#[used] #[no_mangle] pub static mut uci_packet_ready: u8 = 0;
#[used] #[no_mangle] pub static mut uci_status: [u8; 32] = [0; 32];
#[used] #[no_mangle] pub static mut uci_status_length: u8 = 0;
#[used] #[no_mangle] pub static mut uci_packet_last: u8 = 0;
#[used] #[no_mangle] pub static mut uci_response_length: u16 = 0;
static mut COMMAND: *const u8 = null();
static mut RESPONSE: *mut u8 = null_mut();
static mut COMMAND_LENGTH: u8 = 0;
static mut COMMAND_AT: u8 = 0;
static mut RESPONSE_CAPACITY: u16 = 0;
static mut PHASE: u8 = 0;
static mut STREAMING: u8 = 0;
static mut STARTED: u32 = 0;

#[no_mangle]
#[inline(never)]
pub unsafe extern "C" fn uci_start(bytes: *const u8, length: u16,
    out: *mut u8, capacity: u16, stream: u8) -> u8 {
    if uci_busy != 0 || length < 2 || length > 112 { return 0; }
    COMMAND = bytes; COMMAND_LENGTH = length as u8; COMMAND_AT = 0;
    RESPONSE = out; RESPONSE_CAPACITY = capacity; STREAMING = stream;
    uci_error = 0; uci_complete = 0; uci_packet_ready = 0; uci_packet_last = 0;
    uci_status_length = 0; uci_response_length = 0; PHASE = 0;
    STARTED = timer_read(); uci_busy = 1; 1
}

#[no_mangle]
#[link_section = ".highcode"]
pub unsafe extern "C" fn uci_accept_packet() {
    if uci_packet_ready == 0 { return; }
    write(0xdf1c, 2); // DATA_ACC only; no DMA/trigger/IRQ.
    uci_packet_ready = 0; PHASE = 2;
}

#[no_mangle]
#[link_section = ".highcode"]
pub unsafe extern "C" fn uci_step() {
    if uci_busy == 0 || uci_packet_ready != 0 { return; }
    if read(0xdf1d) != 0xc9 { uci_error = UCI_ABSENT; return; }
    if STARTED.wrapping_sub(timer_read()) > 30_000_000 { uci_error = UCI_TIMEOUT; }
    let mut status = read(0xdf1c);
    if status & 8 != 0 { uci_error = UCI_PROTOCOL; write(0xdf1c, 8); }
    if PHASE == 0 {
        if status & 0x3f != 0 { return; }
        let mut budget = 16;
        while budget != 0 && COMMAND_AT < COMMAND_LENGTH {
            write(0xdf1d, *COMMAND.add(COMMAND_AT as usize));
            COMMAND_AT += 1; budget -= 1;
        }
        if COMMAND_AT == COMMAND_LENGTH { write(0xdf1c, 1); PHASE = 1; }
        return;
    }
    if PHASE == 2 {
        if status & 3 != 0 { return; }
        if uci_packet_last != 0 {
            if status & 0x30 != 0 { return; }
            uci_busy = 0; uci_complete = 1; return;
        }
        if status & 0x30 < 0x20 { return; }
        if STREAMING != 0 { uci_response_length = 0; }
        uci_status_length = 0; PHASE = 1;
    }
    if status & 1 != 0 || status & 0x30 < 0x20 { return; }
    let mut budget = 32;
    while budget != 0 {
        status = read(0xdf1c);
        if status & 0xc0 == 0 { break; }
        budget -= 1;
        if status & 0x80 != 0 {
            let value = read(0xdf1e);
            if uci_response_length < RESPONSE_CAPACITY {
                *RESPONSE.add(uci_response_length as usize) = value;
                uci_response_length += 1;
            } else { uci_error = UCI_OVERFLOW; }
        } else {
            let value = read(0xdf1f);
            if uci_status_length < 32 {
                *addr_of_mut!(uci_status).cast::<u8>().add(uci_status_length as usize) = value;
                uci_status_length += 1;
            } else { uci_error = UCI_OVERFLOW; }
        }
    }
    status = read(0xdf1c);
    if status & 0xc0 != 0 { return; }
    uci_packet_last = ((status & 0x30) == 0x20) as u8;
    uci_packet_ready = 1;
    if STREAMING == 0 { uci_accept_packet(); }
}

#[no_mangle]
#[inline(never)]
#[link_section = ".lowcode"]
pub unsafe extern "C" fn uci_status_ok() -> u8 {
    (uci_status_length >= 3 && uci_status[0] == b'0' &&
        uci_status[1] == b'0' && uci_status[2] == b',') as u8
}

// Native LOAD_REU reply: "$%6x BYTES LOADED TO REU $%6x". Compare six
// nibbles without a wide arithmetic operation for every input character.
#[inline(never)]
#[link_section = ".midcode"]
unsafe fn hex_matches(p: *const u8, expected: *const u8) -> bool {
    let mut seen = false;
    let mut i = 0u8;
    while i < 6 {
        let c = *p.add(i as usize);
        let n;
        if c == b' ' { if seen { return false; } n = 0; }
        else {
            seen = true;
            let digit = c.wrapping_sub(b'0');
            n = if digit < 10 { digit } else {
                let alpha = (c | 0x20).wrapping_sub(b'a');
                if alpha >= 6 { return false; }
                alpha+10
            };
        }
        let mut wanted = *expected.add(((5-i) >> 1) as usize);
        if i & 1 == 0 { wanted >>= 4; }
        if n != wanted & 15 { return false; }
        i += 1;
    }
    seen && *expected.add(3) == 0
}

#[no_mangle]
#[inline(never)]
#[link_section = ".midcode"]
pub unsafe extern "C" fn uci_transfer_matches(count: u32, destination: u32) -> u8 {
    let middle = b" BYTES LOADED TO REU $";
    if uci_response_length != 35 || *RESPONSE != b'$' { return 0; }
    let mut i = 0u8;
    while i < 22 {
        if *RESPONSE.add((i+7) as usize) != *middle.as_ptr().add(i as usize) { return 0; }
        i += 1;
    }
    (hex_matches(RESPONSE.add(1), core::ptr::addr_of!(count).cast()) &&
        hex_matches(RESPONSE.add(29), core::ptr::addr_of!(destination).cast())) as u8
}
