//! The machine is owned by the single foreground loop; IRQs remain disabled.
//! Raw pointers are confined to fixed RAM, MMIO, and assembly interfaces.
use core::ptr::{read_volatile, write_volatile};

pub const SAMPLE_RATE: u32 = 16000;
pub const RING_ADDRESS: u32 = 0xfe0000;
pub const RING_BYTES: u32 = 65536;
#[used] #[no_mangle]
pub static mut sampler_ticks_per_ring: u32 = 0;

#[inline(always)]
pub unsafe fn ram_ptr(address: u16) -> *mut u8 { address as *mut u8 }
#[inline(always)]
pub unsafe fn read(address: u16) -> u8 { read_volatile(ram_ptr(address)) }
#[inline(always)]
pub unsafe fn write(address: u16, value: u8) { write_volatile(ram_ptr(address), value); }
#[inline(always)]
unsafe fn word(address: u16, value: u16) {
    write(address, value as u8); write(address + 1, (value >> 8) as u8);
}
extern "C" { pub fn timer_read() -> u32; }
extern "C" {
    // An opaque foreign call may read/write the escaped RAM allocation. A
    // Rust volatile MMIO write alone cannot model the REU's memory side effects.
    fn reu_execute(command: u8, ram: *mut u8);
}

#[inline(never)]
#[link_section = ".highcode"]
unsafe fn transfer(reu: u32, ram: u16, count: u16, command: u8) {
    word(0xdf02, ram); write(0xdf04, reu as u8); write(0xdf05, (reu >> 8) as u8);
    write(0xdf06, (reu >> 16) as u8); word(0xdf07, count);
    write(0xdf09, 0); write(0xdf0a, 0);
    reu_execute(command, ram_ptr(ram));
}
#[no_mangle]
#[inline(never)]
#[link_section = ".midcode"]
pub unsafe extern "C" fn reu_fetch(source: u32, destination: *mut u8, count: u16) {
    if count != 0 { transfer(source, destination as u16, count, 0x91); }
}
#[no_mangle]
#[inline(never)]
#[link_section = ".midcode"]
pub unsafe extern "C" fn reu_stash(source: *const u8, destination: u32, count: u16) {
    if count != 0 { transfer(destination, source as u16, count, 0x90); }
}
#[no_mangle]
#[link_section = ".highcode"]
pub unsafe extern "C" fn raster_line() -> u16 {
    loop {
        let high = read(0xd011) & 128;
        let low = read(0xd012);
        if high == read(0xd011) & 128 { return low as u16 | ((high as u16) << 1); }
    }
}
#[no_mangle]
#[inline(never)]
#[link_section = ".disk_init"]
pub unsafe extern "C" fn platform_init() {
    // startup.S established SEI/CLD before the CRT; never enable either here.
    write(1, 0x35); write(0xdc0d, 0x7f); write(0xdd0d, 0x7f);
    let _ = read(0xdc0d); let _ = read(0xdd0d);
    write(0xd01a, 0); write(0xd019, 255); write(0xd011, 0); write(0xd015, 0); write(0xd031, 0x8f);
    word(0xdd04, 65535); word(0xdd06, 65535); write(0xdd0e, 0x11); write(0xdd0f, 0x51);
}
#[no_mangle]
pub unsafe extern "C" fn sampler_stop() {
    write(0xdf21, 0); write(0xdf41, 0); write(0xdf20, 0); write(0xdf40, 0); write(0xdf3f, 255);
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn be24(address: u16, value: u32) {
    write(address, (value >> 16) as u8); write(address + 1, (value >> 8) as u8); write(address + 2, value as u8);
}
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn configure(divider: u16, length: u32, repeat: u8) {
    sampler_stop();
    let mut voice = 0xdf20;
    while voice <= 0xdf40 {
        write(voice + 1, 0); write(voice + 2, if voice == 0xdf20 { 0 } else { 15 });
        write(voice + 4, 1); be24(voice + 5, RING_ADDRESS + (voice == 0xdf40) as u32);
        be24(voice + 9, length); word_be(voice + 14, divider);
        be24(voice + 17, 0); be24(voice + 21, if repeat != 0 { length } else { 0 });
        voice += 32;
    }
}
#[inline(always)]
unsafe fn word_be(address: u16, value: u16) { write(address, (value >> 8) as u8); write(address + 1, value as u8); }
#[inline(never)]
#[link_section = ".disk_init"]
unsafe fn measure(divider: u16, frames: u16) -> u32 {
    configure(divider, frames as u32 * 2, 0);
    let begin = timer_read(); write(0xdf20, 0x45); write(0xdf40, 0x41);
    loop {
        let elapsed = begin.wrapping_sub(timer_read());
        if elapsed > 1500000 { sampler_stop(); return 0; }
        if read(0xdf20) != 0 { sampler_stop(); return elapsed; }
    }
}
#[no_mangle]
#[inline(never)]
#[link_section = ".disk_init"]
pub unsafe extern "C" fn sampler_calibrate() -> u16 {
    const TARGET: u32 = ((12288u64 * 1022727 + SAMPLE_RATE as u64 / 2) / SAMPLE_RATE as u64) as u32;
    const INITIAL: u16 = (6250000 / SAMPLE_RATE) as u16;
    let mut divider = INITIAL;
    let a = measure(divider, 4096); let b = measure(divider, 16384);
    if a == 0 || b <= a { return 0; }
    let mut ticks = b - a;
    let best = if ticks > TARGET { ticks - TARGET } else { TARGET - ticks };
    let candidate = ((divider as u32 * TARGET + ticks / 2) / ticks) as u16;
    if candidate < INITIAL - 16 || candidate > INITIAL + 16 { return 0; }
    if candidate != divider {
        let a = measure(candidate, 4096); let b = measure(candidate, 16384);
        if a == 0 || b <= a { return 0; }
        let diff = if b - a > TARGET { b - a - TARGET } else { TARGET - (b - a) };
        if diff < best { divider = candidate; ticks = b - a; }
    }
    sampler_ticks_per_ring = (ticks * 8 + 1) / 3;
    divider
}
#[no_mangle]
#[inline(never)]
#[link_section = ".disk_init"]
pub unsafe extern "C" fn sampler_prepare(divider: u16) { configure(divider, RING_BYTES, 1); }
#[no_mangle]
pub unsafe extern "C" fn sampler_volume(level: u8) { write(0xdf21, level); write(0xdf41, level); }
#[no_mangle]
pub unsafe extern "C" fn sampler_arm() {
    sampler_volume(32); write(0xdf20, 0x43); write(0xdf40, 0x43);
}
