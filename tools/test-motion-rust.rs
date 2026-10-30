//! Independent easing and transport oracle for the production motion module.
#![allow(dead_code, non_upper_case_globals)]
#[path = "../target-c64/src/motion.rs"] mod motion;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

macro_rules! get { ($field:ident) => { read_volatile(addr_of!(motion::rotation_state.$field)) }; }
macro_rules! set { ($field:ident, $value:expr) => { write_volatile(addr_of_mut!(motion::rotation_state.$field), $value) }; }

fn accumulated_weight(n: u32, duration: u32) -> u32 {
    if n <= 30 { n*(n+1)/2 }
    else if n <= duration-30 { 465+30*(n-30) }
    else { let remaining=duration-n; 30*(duration-29)-remaining*(remaining+1)/2 }
}

unsafe fn request(target: i8, duration: u16) {
    set!(request_target, target); set!(request_duration, duration); set!(request_kind, 1);
}

#[test]
fn motion_state_has_exact_32_byte_diagnostic_abi() {
    use motion::MotionState;
    assert_eq!(core::mem::size_of::<MotionState>(), 32);
    assert_eq!(core::mem::offset_of!(MotionState, rng), 0);
    assert_eq!(core::mem::offset_of!(MotionState, elapsed), 2);
    assert_eq!(core::mem::offset_of!(MotionState, duration), 4);
    assert_eq!(core::mem::offset_of!(MotionState, denominator), 6);
    assert_eq!(core::mem::offset_of!(MotionState, accumulator), 8);
    assert_eq!(core::mem::offset_of!(MotionState, distance), 10);
    assert_eq!(core::mem::offset_of!(MotionState, direction), 11);
    assert_eq!(core::mem::offset_of!(MotionState, target), 12);
    assert_eq!(core::mem::offset_of!(MotionState, next_k), 13);
    assert_eq!(core::mem::offset_of!(MotionState, pending), 14);
    assert_eq!(core::mem::offset_of!(MotionState, kind), 15);
    assert_eq!(core::mem::offset_of!(MotionState, next_accumulator), 16);
    assert_eq!(core::mem::offset_of!(MotionState, next_elapsed), 18);
    assert_eq!(core::mem::offset_of!(MotionState, legs), 20);
    assert_eq!(core::mem::offset_of!(MotionState, commits), 22);
    assert_eq!(core::mem::offset_of!(MotionState, stalls), 24);
    assert_eq!(core::mem::offset_of!(MotionState, request_target), 26);
    assert_eq!(core::mem::offset_of!(MotionState, request_kind), 27);
    assert_eq!(core::mem::offset_of!(MotionState, request_duration), 28);
    assert_eq!(core::mem::offset_of!(MotionState, reserved), 30);
}

#[test]
fn exact_easing_random_targets_stalls_cancel_and_flatten() { unsafe {
    let mut legs=0;
    // Closed-form cumulative triangular/constant/triangular weights are
    // independent of the runtime's per-frame remainder accumulator.
    for start in -72i16..=72 { for target in -72i16..=72 {
        for duration in [180u16,181,240,359,360] {
            motion::motion_init(0x6400); request(target as i8,duration);
            let mut current=start as i8;
            for frame in 1..=duration {
                let next=motion::motion_propose(current,0);
                let distance=(target-start).unsigned_abs() as u32;
                let steps=distance*accumulated_weight(frame as u32,duration as u32)/(30*(duration as u32-29));
                let expected=start+(target-start).signum()*steps as i16;
                assert_eq!(next as i16,expected,"easing start={start} target={target} duration={duration} frame={frame}");
                assert!((next as i16-current as i16).abs()<=1);
                assert_eq!(get!(elapsed),frame-1);
                assert_eq!(get!(duration),duration);
                motion::motion_commit(); current=next;
            }
            assert_eq!(current as i16,target); assert_eq!(get!(elapsed),duration);
            assert_eq!(get!(accumulator),0); assert_eq!(get!(legs),1);
            legs+=1;
        }
    }}
    // Identical seeds reproduce both random endpoints and180..360-frame legs.
    let mut first_run=Vec::new();
    for pass in 0..2 {
        motion::motion_init(0x6510); let mut current=0i8;
        for leg in 0..512 {
            let initial=current; let first=motion::motion_propose(current,0);
            let target=get!(target); let duration=get!(duration);
            assert!((-72..=72).contains(&target)); assert!((180..=360).contains(&duration));
            assert!((target as i16-initial as i16).abs()>=16);
            if pass==0 { first_run.push((target,duration)); }
            else { assert_eq!((target,duration),first_run[leg]); }
            motion::motion_commit(); current=first;
            for _ in 1..duration { current=motion::motion_propose(current,0); motion::motion_commit(); }
            assert_eq!(current,target);
        }
    }
    motion::motion_init(0); assert_eq!(get!(rng),1);
    request(72,180); let mut current=0;
    for _ in 0..60 { current=motion::motion_propose(current,0); motion::motion_commit(); }
    let next=motion::motion_propose(current,0);
    let frozen=(get!(rng),get!(elapsed),get!(accumulator),get!(commits),get!(legs),get!(next_elapsed),get!(next_accumulator));
    for _ in 0..300 { assert_eq!(motion::motion_propose(current,0),next); }
    assert_eq!((get!(rng),get!(elapsed),get!(accumulator),get!(commits),get!(legs),get!(next_elapsed),get!(next_accumulator)),frozen);
    assert_eq!(get!(stalls),300);
    motion::motion_cancel(); assert_eq!(get!(pending),0);
    assert_eq!(motion::motion_propose(current,0),next);
    motion::motion_commit(); assert_eq!(get!(elapsed),frozen.1+1);
    let committed=get!(commits); motion::motion_commit(); assert_eq!(get!(commits),committed);
    // A pending proposal remains stable even if the caller changes flatten.
    let pending=motion::motion_propose(next,0);
    assert_eq!(motion::motion_propose(next,1),pending);
    motion::motion_cancel();
    for start in -72i16..=72 {
        motion::motion_init(0x1234); let mut current=start as i8; let mut frames=0;
        while current!=0 {
            let next=motion::motion_propose(current,1);
            assert!((next as i16).abs()<(current as i16).abs());
            assert!((next as i16-current as i16).abs()<=2);
            motion::motion_commit(); current=next; frames+=1;
            assert_eq!(get!(duration),0);
        }
        assert_eq!(frames,(start.abs()+1)/2); assert!(frames<=36);
    }
    for target in [-128i8,-73,-72,0,72,73,127] { for duration in [0u16,179,180,360,361,65535] {
        motion::motion_init(7); request(target,duration); motion::motion_propose(0,0);
        assert_eq!(get!(target),target.clamp(-72,72)); assert_eq!(get!(duration),duration.clamp(180,360));
        assert_eq!(get!(request_kind),0);
    }}
    // Diagnostic requests wait for the next natural boundary, never changing
    // an in-progress leg or the already-published angle.
    motion::motion_init(9); request(72,180); let mut current=motion::motion_propose(0,0); motion::motion_commit();
    request(-72,360);
    for _ in 1..180 { current=motion::motion_propose(current,0); assert_eq!(get!(target),72); motion::motion_commit(); }
    assert_eq!(current,72); motion::motion_propose(current,0);
    assert_eq!(get!(target),-72); assert_eq!(get!(duration),360);
    println!("Motion: {legs} exact eased legs,1024 deterministic random legs,300 stalled proposals,145 flatten origins,42 bounded diagnostic requests; hardware_measured=false");
} }
