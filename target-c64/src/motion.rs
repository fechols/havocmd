//! Integer, eased random rotation. A proposed frame advances only after the
//! panorama cache commits that same camera/angle state.
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

pub const MAX_K: i8 = 72;

#[repr(C)]
pub struct MotionState {
    pub rng: u16,
    pub elapsed: u16,
    pub duration: u16,
    pub denominator: u16,
    pub accumulator: u16,
    pub distance: u8,
    pub direction: i8,
    pub target: i8,
    pub next_k: i8,
    pub pending: u8,
    pub kind: u8,
    pub next_accumulator: u16,
    pub next_elapsed: u16,
    pub legs: u16,
    pub commits: u16,
    pub stalls: u16,
    // A data-only diagnostic request, consumed at a natural leg boundary.
    // The instruction model changes these fields, never the rendered pose.
    pub request_target: i8,
    pub request_kind: u8,
    pub request_duration: u16,
    pub reserved: u16,
}

#[used]
#[no_mangle]
#[link_section = ".motion_state"]
pub static mut rotation_state: MotionState = MotionState {
    rng: 0, elapsed: 0, duration: 0, denominator: 0, accumulator: 0,
    distance: 0, direction: 0, target: 0, next_k: 0, pending: 0, kind: 0,
    next_accumulator: 0, next_elapsed: 0, legs: 0, commits: 0, stalls: 0,
    request_target: 0, request_kind: 0, request_duration: 0, reserved: 0,
};

#[no_mangle]
#[inline(never)]
#[link_section = ".disk_init"]
pub unsafe extern "C" fn motion_init(seed: u16) {
    let p = addr_of_mut!(rotation_state).cast::<u8>();
    let mut n = 0u8;
    while n < 32 { write_volatile(p.add(n as usize), 0); n += 1; }
    rotation_state.rng = if seed == 0 { 1 } else { seed };
}

#[inline(never)]
#[cfg_attr(not(test), link_section = ".bankcode")]
unsafe fn random_byte() -> u8 {
    let mut n = 0;
    while n < 8 {
        let value = rotation_state.rng;
        rotation_state.rng = (value >> 1) ^ if value & 1 != 0 { 0xb400 } else { 0 };
        n += 1;
    }
    (rotation_state.rng >> 8) as u8
}

#[inline(never)]
#[cfg_attr(not(test), link_section = ".bankcode")]
unsafe fn new_leg(current: i8) {
    let mut target = current;
    let mut tries = 0u8;
    while tries < 4 {
        target = (((random_byte() as u16 * 145) >> 8) as i16 - 72) as i8;
        let difference = target as i16 - current as i16;
        if difference <= -16 || difference >= 16 { break; }
        tries += 1;
    }
    if tries == 4 { target = if current >= 0 { -MAX_K } else { MAX_K }; }
    let mut duration = 180 + ((random_byte() as u16 * 181) >> 8);
    if read_volatile(addr_of!(rotation_state.request_kind)) == 1 {
        target = read_volatile(addr_of!(rotation_state.request_target)).clamp(-MAX_K, MAX_K);
        duration = read_volatile(addr_of!(rotation_state.request_duration)).clamp(180, 360);
        write_volatile(addr_of_mut!(rotation_state.request_kind), 0);
    }
    let distance = target as i16 - current as i16;
    rotation_state.target = target;
    rotation_state.direction = if distance < 0 { -1 } else { 1 };
    rotation_state.distance = if distance < 0 { -distance } else { distance } as u8;
    rotation_state.duration = duration;
    rotation_state.denominator = 30 * (duration - 29);
    rotation_state.elapsed = 0;
    rotation_state.accumulator = 0;
    rotation_state.legs = rotation_state.legs.wrapping_add(1);
}

/// Return one prepared angle step. Repeated calls during a cache stall return
/// the identical proposal without consuming time or accumulating extra steps.
#[cfg_attr(not(test), export_name = "motion_propose_banked")]
#[cfg_attr(not(test), link_section = ".bankcode")]
#[inline(never)]
pub unsafe extern "C" fn motion_propose(current: i8, flatten: u8) -> i8 {
    if rotation_state.pending != 0 {
        rotation_state.stalls = rotation_state.stalls.wrapping_add(1);
        return rotation_state.next_k;
    }
    rotation_state.kind = (flatten != 0) as u8;
    let next;
    if flatten != 0 {
        next = if current > 2 { current - 2 }
            else if current < -2 { current + 2 } else { 0 };
    } else {
        if rotation_state.duration == 0 || rotation_state.elapsed == rotation_state.duration {
            new_leg(current);
        }
        let elapsed = rotation_state.elapsed + 1;
        let weight = elapsed.min(rotation_state.duration + 1 - elapsed).min(30);
        let mut accumulator = rotation_state.accumulator + rotation_state.distance as u16 * weight;
        next = if accumulator >= rotation_state.denominator {
            accumulator -= rotation_state.denominator;
            current + rotation_state.direction
        } else { current };
        rotation_state.next_accumulator = accumulator;
        rotation_state.next_elapsed = elapsed;
    }
    rotation_state.next_k = next;
    rotation_state.pending = 1;
    next
}

#[cfg_attr(not(test), export_name = "motion_commit_banked")]
#[cfg_attr(not(test), link_section = ".bankcode")]
#[inline(never)]
pub unsafe extern "C" fn motion_commit() {
    if rotation_state.pending == 0 { return; }
    if rotation_state.kind != 0 {
        rotation_state.duration = 0;
    } else {
        rotation_state.accumulator = rotation_state.next_accumulator;
        rotation_state.elapsed = rotation_state.next_elapsed;
    }
    rotation_state.pending = 0;
    rotation_state.commits = rotation_state.commits.wrapping_add(1);
}

#[no_mangle]
#[inline(never)]
#[link_section = ".highcode"]
pub unsafe extern "C" fn motion_cancel() { rotation_state.pending = 0; }
