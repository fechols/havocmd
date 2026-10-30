// Host oracle for the production rectangular streamer; no hardware access.
#![allow(static_mut_refs,dead_code)]
#[no_mangle] pub static mut havoc_raster_x:u8=0;
#[no_mangle] pub static mut havoc_raster_y:u8=0;
mod platform {
    pub static mut RAM:[u8;65536]=[0;65536];
    pub static mut REU:[u8;2*1024*1024]=[0;2*1024*1024];
    pub static mut TICKS:u32=0xf0000000;
    pub static mut TRANSFERS:u32=0;
    pub unsafe fn ram_ptr(at:u16)->*mut u8 {core::ptr::addr_of_mut!(RAM).cast::<u8>().add(at as usize)}
    pub unsafe fn timer_read()->u32 {TICKS=TICKS.wrapping_sub(1);TICKS}
    pub unsafe fn reu_fetch(at:u32,dst:*mut u8,count:u16) {
        let base=ram_ptr(0) as usize;
        assert!(at as usize+count as usize<=2*1024*1024);
        assert!(dst as usize>=base&&dst as usize+count as usize<=base+65536);
        core::ptr::copy_nonoverlapping(core::ptr::addr_of!(REU).cast::<u8>().add(at as usize),dst,count as usize);
        TICKS=TICKS.wrapping_sub(count as u32+12);TRANSFERS+=1;
    }
}
mod panorama {
    include!("../target-c64/src/panorama.rs");
    #[cfg(test)] mod tests {
        use super::*;
        use crate::platform::{RAM,REU,TICKS,TRANSFERS};
        static mut FRAMES:u32=0;
        fn pixel(pic:u32,x:u32,y:u32)->u8 {
            let (w,h)=if pic==2 {(905,1086)}else{(640,768)};
            let x=x.min(w-1);let y=y.min(h-1);
            ((x*13)^(y*7)^(x>>4)^(x>>8)^(y>>6)^(pic*11)) as u8&15
        }
        unsafe fn asset(address:usize,pic:u32,w:u32,h:u32) {
            let sx=((w-256)/2)&!15;let sy=((h-256)/2)&!7;let mut at=address;
            for x in (0..256).step_by(2) {for y in 0..256 {
                REU[at]=pixel(pic,sx+x,sy+y)|pixel(pic,sx+x+1,sy+y)<<4;at+=1;}}
            for tx in (0..((w+15)&!15)).step_by(16) {for ty in (0..((h+7)&!7)).step_by(8) {
                for x in (0..16).step_by(2) {for y in 0..8 {
                    REU[at]=pixel(pic,tx+x,ty+y)|pixel(pic,tx+x+1,ty+y)<<4;at+=1;}}}}
        }
        unsafe fn shown(x:u32,y:u32)->u8 {
            let px=havoc_raster_x.wrapping_add(x as u8);let py=havoc_raster_y.wrapping_add(y as u8);
            let b=RAM[0x4000+(px as usize/2)*256+py as usize];if px&1==0 {b&15}else{b>>4}
        }
        unsafe fn frame(pic:u32) {
            assert!(pano_camera_x+200<=ACTIVE.w&&pano_camera_y+240<=ACTIVE.h);
            for y in 0..240 {for x in 0..200 {
                assert_eq!(shown(x,y),pixel(pic,pano_camera_x as u32+x,pano_camera_y as u32+y),
                    "pic{pic} camera{},{} origin{},{} horizontal{HORIZONTAL} x{x} y{y}",pano_camera_x,pano_camera_y,ORIGIN_X,ORIGIN_Y);
            }}FRAMES+=1;
        }
        unsafe fn service(jobs:u8) {
            let before=TRANSFERS;let normal=PHASE==0;
            assert!(pano_step(jobs,TICKS.wrapping_sub(50000))<=jobs);
            assert!(TRANSFERS-before<=(if normal {1}else{2})*jobs as u32,"DMA count exceeded job bound");
        }
        unsafe fn travel(pic:u32,count:u32,dx:i8,dy:i8,jobs:u8,delayed:bool)->u32 {
            let mut held=0;
            for i in 0..count {
                if i&1==0 {pano_request(dx,dy);}
                let expected=(moved(pano_camera_x,REQUESTED_X,ACTIVE.w-200),moved(pano_camera_y,REQUESTED_Y,ACTIVE.h-240));
                pano_tick();if (pano_camera_x,pano_camera_y)!=expected {held+=1;}
                frame(pic);
                if !delayed||i%19>=7 {service(jobs);frame(pic);}
            }held
        }
        unsafe fn transition(parity:u8) {
            while havoc_raster_x&1!=parity {pano_request(1,0);pano_tick();service(6);}
            let old_x=pano_camera_x;let old_y=pano_camera_y;
            assert_eq!(pano_prefetch(0x80000,640,768),1);assert_eq!(pano_prefetch(0x80000,640,768),0);
            pano_transition();assert_eq!(PHASE,1);
            for _ in 0..300 {if pano_can_transition()!=0 {break;}service(6);frame(0);}
            assert_eq!(pano_can_transition(),1);pano_transition();
            for i in 0..700 {
                if pano_transition_done()!=0 {break;}pano_tick();if i%17>=5 {service(6);}
                if pano_transition_done()!=0 {break;}
                for y in 0..240 {for x in 0..200 {
                    let relative=SLIDE as u32+x;
                    let want=if relative<SLIDE_LENGTH as u32 {pixel(0,old_x as u32+relative,old_y as u32+y)}
                        else {pixel(1,NEXT.sx as u32+relative-SLIDE_LENGTH as u32,NEXT.sy as u32+y)};
                    assert_eq!(shown(x,y),want,"slide parity{parity} x{x} y{y}");}}
            }
            assert_eq!(pano_transition_done(),1);assert_eq!(pano_busy(),0);assert_eq!(BAND_REPAIR,0);frame(1);
            travel(1,400,1,1,6,false);travel(1,400,-1,-1,6,false);
        }
        unsafe fn check_band(b:u8,pic:u32) {
            assert_eq!(BAND_COLUMNS[b as usize],16);
            for c in 0..128u16 {let x=ORIGIN_X+c*2;let packed=(x as u8).wrapping_sub(BIAS_X)/2;
                for r in 0..8u16 {
                    let got=RAM[(BANDS+((b as u16)<<10)+(r<<7)+packed as u16) as usize];
                    let y=BAND_Y[b as usize]+r;
                    assert_eq!(got,pixel(pic,x as u32,y as u32)|pixel(pic,x as u32+1,y as u32)<<4,
                        "repaired band{b} x{x} y{y}");
                }
            }
        }
        unsafe fn finish_right_strip() {
            for _ in 0..16 {pano_request(1,0);pano_tick();}
            // This fixture isolates band repair from vertical preparation.
            DIRECTION_Y=0;
            let old=ORIGIN_X;for _ in 0..40 {service(1);frame(0);if ORIGIN_X!=old {break;}}
            assert_eq!(ORIGIN_X,old+16);assert_eq!(HORIZONTAL,0);
        }
        unsafe fn repair_contract() {
            assert_eq!(pano_init(0,640,768),1);pano_request(0,1);service(32);
            assert_eq!(BAND_COLUMNS,[16,16]);let tags=BAND_Y;
            finish_right_strip();assert_eq!(BAND_REPAIR,3);assert_eq!(BAND_COLUMNS,[16,16]);assert_eq!(BAND_Y,tags);
            let source=STRIP_X;let origin=ORIGIN_Y;let transfers=TRANSFERS;
            // Reversal would immediately start another strip if repairs did
            // not have priority; a vertical row may not consume stale data.
            pano_request(-1,1);service(1);assert_eq!(BAND_REPAIR,2);check_band(0,0);
            assert_eq!((STRIP_X,ORIGIN_Y,HORIZONTAL),(source,origin,0));
            service(1);assert_eq!(BAND_REPAIR,0);check_band(1,0);assert_eq!(TRANSFERS-transfers,2);
            assert_eq!((STRIP_X,ORIGIN_Y,HORIZONTAL),(source,origin,0));frame(0);
            assert_eq!(pano_init(0,640,768),1);pano_request(0,1);service(19);
            assert_eq!(BAND_COLUMNS,[16,3]);let tags=BAND_Y;finish_right_strip();
            assert_eq!(BAND_REPAIR,1);assert_eq!(BAND_COLUMNS,[16,2]);assert_eq!(BAND_Y,tags);
            service(1);check_band(0,0);assert_eq!(BAND_REPAIR,0);
            // A song change can supersede unperformed repairs; its final seed
            // adoption must discard every old picture's band/repair tag.
            assert_eq!(pano_init(0,640,768),1);pano_request(0,1);service(32);finish_right_strip();
            assert_eq!(BAND_REPAIR,3);transition(0);
        }
        unsafe fn earliest_strip_contract() {
            for left in [false,true] {
                assert_eq!(pano_init(0,640,768),1);DIRECTION_X=if left {-1}else{1};DIRECTION_Y=0;
                let origin=ORIGIN_X;pano_camera_x=origin+if left {41}else{15};publish_camera();
                assert_eq!(start_strip(),0,"strip began while outgoing pixels were visible");frame(0);
                pano_camera_x=origin+if left {40}else{16};publish_camera();
                assert_eq!(start_strip(),1,"first safe strip position did not start prefetch");
                for _ in 0..40 {service(1);frame(0);if ORIGIN_X!=origin {break;}}
                assert_eq!(ORIGIN_X,if left {origin-16}else{origin+16});frame(0);
            }
        }
        unsafe fn partial_prefix_contract() {
            for left in [false,true] {for a in 0..=16u8 {for b in 0..=16u8 {
                assert_eq!(pano_init(0,640,768),1);BAND_Y=[512,520];
                for _ in 0..a {band_job(0);}for _ in 0..b {band_job(1);}
                let old=ORIGIN_X;ORIGIN_X=if left {old-16}else{old+16};
                STRIP_X=if left {ORIGIN_X}else{old+256};retain_gate();
                let expected=|k:u8|if left {if k==0 {0}else{(k+1).min(16)}}else if k==16 {16}else{k.saturating_sub(1)};
                assert_eq!(BAND_COLUMNS,[expected(a),expected(b)]);assert_eq!(BAND_Y,[512,520]);
                let mask=if a==16||(left&&a!=0) {1}else{0}|if b==16||(left&&b!=0) {2}else{0};
                assert_eq!(BAND_REPAIR,mask);
                let origin=ORIGIN_Y;let source=STRIP_X;let transfers=TRANSFERS;
                DIRECTION_X=if left {1}else{-1};DIRECTION_Y=1;
                while BAND_REPAIR!=0 {service(1);assert_eq!((HORIZONTAL,ORIGIN_Y,STRIP_X),(0,origin,source));}
                assert_eq!(TRANSFERS-transfers,mask.count_ones());
                for slot in 0..2 {for c in 0..BAND_COLUMNS[slot]*8 {
                    let x=ORIGIN_X+c as u16*2;let packed=(x as u8).wrapping_sub(BIAS_X)/2;
                    for r in 0..8 {let y=BAND_Y[slot]+r as u16;
                        let got=RAM[BANDS as usize+slot*1024+r*128+packed as usize];
                        assert_eq!(got,pixel(0,x as u32,y as u32)|pixel(0,x as u32+1,y as u32)<<4,
                            "prefix left{left} counts{a},{b} slot{slot} x{x} y{y}");
                    }
                }}frame(0);
            }}}
        }
        unsafe fn strip_copy_wrap_contract() {
            assert_eq!(pano_init(0,640,768),1);
            // Exhaust every physicalY phase for both clipped edge tiles of
            // every possible8-row alignment. Packed columns also cross127→0.
            BIAS_X=200;STRIP_X=448;STRIP_TILES=255;HORIZONTAL=1;
            for offset in 0..8u16 {ORIGIN_Y=256+offset;STRIP_Y=256;
                for tile in [0,((offset+255)>>3) as u8] {
                    for bias in 0..256u16 {
                        BIAS_Y=bias as u8;STRIP_TILE=tile;RAM[0x4000..0xc000].fill(0xa5);
                        let mut expected=[0xa5u8;32768];let y=STRIP_Y+tile as u16*8;
                        for c in 0..8u16 {for r in 0..8u16 {
                            if y+r>=ORIGIN_Y&&y+r<ORIGIN_Y+256 {
                                let col=(((STRIP_X as u8).wrapping_sub(BIAS_X)/2).wrapping_add(c as u8))&127;
                                let row=(y as u8).wrapping_add(r as u8).wrapping_sub(BIAS_Y);
                                let x=STRIP_X+c*2;expected[col as usize*256+row as usize]=
                                    pixel(0,x as u32,(y+r) as u32)|pixel(0,x as u32+1,(y+r) as u32)<<4;
                            }
                        }}
                        strip_job();assert_eq!(&RAM[0x4000..0xc000],&expected,"strip offset{offset} tile{tile} bias{bias}");
                    }
                }
            }
        }
        // The host clock models DMA and timer calls, not 6502 instructions.
        // Exercise pixel safety when deadline admission divides the cache work;
        // the linked PRG model separately checks CPU timing and smoothness.
        unsafe fn limited_service() {
            let deadline=TICKS.wrapping_sub(400);
            assert!(pano_step(6,deadline)<=6);
            assert!(TICKS.wrapping_sub(deadline) as i32>=0,"mock DMA budget exceeded");
        }
        unsafe fn limited_diagonal() {
            assert_eq!(pano_init(0x100000,905,1086),1);
            let start=(pano_camera_x,pano_camera_y);let mut hold_x=0;let mut hold_y=0;let mut max_x=0;let mut max_y=0;
            for i in 0..600 {
                if i&1==0 {pano_request(1,1);}
                let before=(pano_camera_x,pano_camera_y);pano_tick();frame(2);
                hold_x=if pano_camera_x==before.0 {hold_x+1}else{0};hold_y=if pano_camera_y==before.1 {hold_y+1}else{0};
                max_x=max_x.max(hold_x);max_y=max_y.max(hold_y);limited_service();frame(2);
            }
            let distance=(pano_camera_x-start.0,pano_camera_y-start.1);
            assert!(distance.0>0&&distance.1>0,"stressed streamer stopped making progress");
            println!("DMA-budget safety stress:600frames distance{distance:?}, longest axis holds{max_x},{max_y}; actual PRG timing and smoothness are checked separately.");
        }
        #[test]
        fn rectangular_prefetch_wrap_edges_reversal_and_wipes() {unsafe {
            assert_eq!(core::mem::size_of::<Picture>(),14);
            asset(0,0,640,768);asset(0x80000,1,640,768);asset(0x100000,2,905,1086);
            strip_copy_wrap_contract();
            repair_contract();
            earliest_strip_contract();
            partial_prefix_contract();
            limited_diagonal();
            assert_eq!(pano_init(0,255,768),0);assert_eq!(pano_init(0,640,32760),0);
            assert_eq!(pano_init(0,640,768),1);frame(0);
            // The historical high-water metric is diagnostic only: one slow
            // job cannot permanently block a cheaper band job in later frames.
            pano_request(0,1);pano_max_job_ticks=60000;
            let before=TRANSFERS;let deadline=TICKS.wrapping_sub(122);
            assert_eq!(pano_step(1,deadline),0);assert_eq!(TRANSFERS,before);
            let deadline=TICKS.wrapping_sub(123);
            assert_eq!(pano_step(1,deadline),1);assert_eq!(TRANSFERS,before+1);frame(0);
            pano_max_job_ticks=256;
            let snapshot=RAM[0x4000..0xc000].to_vec();let transfers=TRANSFERS;
            pano_request(1,1);assert_eq!(pano_step(6,TICKS),0);
            assert_eq!(&RAM[0x4000..0xc000],&snapshot);assert_eq!(TRANSFERS,transfers);
            // Proactive work at a half-pixel/frame diagonal must hide cache
            // boundary crossings when six jobs/frame are available.
            let held=travel(0,1500,1,1,6,false);assert_eq!(held,0,"prefetch failed to hide forward streaming");
            assert_eq!((pano_camera_x,pano_camera_y),(440,528));
            let held=travel(0,1500,-1,-1,6,false);assert_eq!(held,0,"prefetch failed to hide reverse streaming");
            assert_eq!((pano_camera_x,pano_camera_y),(0,0));
            TICKS=32;travel(0,1500,1,1,6,true);frame(0);
            // Reversals during an in-flight strip and partly built row bands
            // may pause, but must never expose old cache pixels.
            for round in 0..12 {let d=if round&1==0 {-1}else{1};travel(0,37,d,-d,3,true);}
            assert_eq!(pano_init(0,640,768),1);travel(0,68,1,1,1,false);transition(1);
            assert_eq!(pano_init(0,640,768),1);transition(0);
            assert_eq!(pano_init(0x100000,905,1086),1);
            let held=travel(2,1900,1,1,6,false);assert_eq!(held,0);assert_eq!((pano_camera_x,pano_camera_y),(705,846));
            let held=travel(2,1900,-1,-1,6,false);assert_eq!(held,0);assert_eq!((pano_camera_x,pano_camera_y),(0,0));
            travel(2,1600,1,0,6,false);assert_eq!((pano_camera_x,pano_camera_y),(705,0));
            travel(2,1900,-1,1,6,false);assert_eq!((pano_camera_x,pano_camera_y),(0,846));
            println!("Rust rectangular panorama: {FRAMES} full200x240 original-art frames; proactive half-pixel diagonal streaming without holds at six jobs/frame, all true edges, both wraps, delayed jobs, mid-strip reversals, odd/even stalled song slides, timerwrap and deadline refusal; {TRANSFERS} DMA transfers.");
        }}
    }
}
