#![no_std]
#![no_main]
#![allow(dead_code, non_upper_case_globals, static_mut_refs)]

mod assets { include!(concat!(env!("OUT_DIR"), "/assets.rs")); }
mod platform;
mod audio;
mod uci;
mod musicdisk;
mod panorama;
mod controller;

#[no_mangle]
pub unsafe extern "C" fn main() -> i16 { controller::run() }

#[panic_handler]
#[link_section = ".tailcode"]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // No formatting or unwinding on the target, and no startup-overlay call.
    unsafe { controller::fail(8) }
}
