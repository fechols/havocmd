// Rectangular256x256 cache with proactive horizontal strips and two row bands.
use crate::platform::{ram_ptr,reu_fetch,timer_read};
use core::ptr::addr_of_mut;
extern "C" {static mut havoc_raster_x:u8;static mut havoc_raster_y:u8;}
#[cfg(not(test))] extern "C" {
    fn pano_picture(p:*mut Picture,address:u32,w:u16,h:u16);
    fn pano_retain();
    fn pano_strip()->u8;
    // Opaque calls expose both source reads and destination writes to LLVM.
    fn pano_copy_band(dest:*mut u8,source:*const u8,count:u16);
    fn pano_copy_row(dest:*mut u8,source:*const u8,count:u16);
    fn pano_copy_strip(dest:*mut u8,source:*const u8,count:u16);
}
const CACHE:u16=0x4000;const TILE:u16=0xe6c0;const BANDS:u16=0xe700;const INVALID:u16=65535;
#[repr(C,packed)] #[derive(Clone,Copy)]
struct Picture {reu:u32,w:u16,h:u16,sh:u16,sx:u16,sy:u16}
const EMPTY:Picture=Picture{reu:0,w:0,h:0,sh:0,sx:0,sy:0};
static mut ACTIVE:Picture=EMPTY;static mut NEXT:Picture=EMPTY;
static mut ORIGIN_X:u16=0;static mut ORIGIN_Y:u16=0;
static mut BIAS_X:u8=0;static mut BIAS_Y:u8=0;
static mut BAND_Y:[u16;2]=[INVALID;2];static mut BAND_COLUMNS:[u8;2]=[0;2];
static mut BAND_REPAIR:u8=0;
static mut REQUESTED_X:i8=0;static mut REQUESTED_Y:i8=0;
static mut DIRECTION_X:i8=0;static mut DIRECTION_Y:i8=0;
static mut HORIZONTAL:i8=0;static mut STRIP_X:u16=0;static mut STRIP_Y:u16=0;
static mut STRIP_TILE:u8=0;static mut STRIP_TILES:u8=0;
// Song transition only:0normal,1hidden prefetch,2slide,3hidden tail.
static mut PHASE:u8=0;static mut SEED_COLUMN:u8=0;static mut SEED_PART:u8=0;
static mut START_X:u8=0;static mut START_Y:u8=0;static mut SEAM:u8=0;
static mut SLIDE:u8=0;static mut SLIDE_LENGTH:u8=0;
#[used] #[no_mangle] pub static mut pano_camera_x:u16=0;
#[used] #[no_mangle] pub static mut pano_camera_y:u16=0;
#[used] #[no_mangle] pub static mut pano_max_job_ticks:u16=256;
#[used] #[no_mangle] pub static mut pano_prefetched:u8=0;
#[used] #[no_mangle] pub static mut pano_retired:u8=0;
#[inline(always)] fn valid(w:u16,h:u16)->bool {w>=256&&h>=256&&w<32752&&h<32760}
#[cfg_attr(not(test),export_name="pano_picture_banked")]
#[cfg_attr(not(test),link_section=".bankcode")]
#[inline(never)] unsafe extern "C" fn picture(p:*mut Picture,address:u32,w:u16,h:u16) {
    (*p).reu=address;(*p).w=w;(*p).h=h;(*p).sh=(h+7)&65528;
    (*p).sx=((w-256)/2)&65520;(*p).sy=((h-256)/2)&65528;
}
#[inline(always)] unsafe fn picture_gate(p:*mut Picture,address:u32,w:u16,h:u16) {
    #[cfg(test)] {picture(p,address,w,h)} #[cfg(not(test))] {pano_picture(p,address,w,h)}
}
#[inline(always)] unsafe fn invalidate_bands() {BAND_Y=[INVALID;2];BAND_COLUMNS=[0;2];BAND_REPAIR=0;}
// Bands use physical cache-column positions. A right shift drops the first
// prefix column; a left shift adds a repaired leading column. Keep every
// remaining valid tile, including useful work in partially prefetched bands.
#[inline(always)] unsafe fn retain_gate() {
    #[cfg(test)] {retain_bands()} #[cfg(not(test))] {pano_retain()}
}
#[cfg_attr(not(test),export_name="pano_retain_banked")]
#[cfg_attr(not(test),link_section=".bankcode")]
#[inline(never)] unsafe extern "C" fn retain_bands() {
    BAND_REPAIR=0;let left=STRIP_X==ORIGIN_X;
    if BAND_Y[0]!=INVALID {
        if BAND_COLUMNS[0]==16 {BAND_REPAIR=1;}
        else if BAND_COLUMNS[0]!=0 {if left {BAND_COLUMNS[0]+=1;BAND_REPAIR=1;}else{BAND_COLUMNS[0]-=1;}}
    }else{BAND_COLUMNS[0]=0;}
    if BAND_Y[1]!=INVALID {
        if BAND_COLUMNS[1]==16 {BAND_REPAIR|=2;}
        else if BAND_COLUMNS[1]!=0 {if left {BAND_COLUMNS[1]+=1;BAND_REPAIR|=2;}else{BAND_COLUMNS[1]-=1;}}
    }else{BAND_COLUMNS[1]=0;}
}
#[link_section=".highcode"] #[inline(never)] unsafe fn publish_camera() {
    havoc_raster_x=(pano_camera_x as u8).wrapping_sub(BIAS_X);
    havoc_raster_y=(pano_camera_y as u8).wrapping_sub(BIAS_Y);
}
#[inline(always)] fn moved(at:u16,d:i8,maximum:u16)->u16 {
    if d<0 {if at!=0 {at-1}else{0}}else if d>0&&at<maximum {at+1}else{at}
}
#[link_section=".disk_init"] #[inline(never)] unsafe fn initialize(address:u32,w:u16,h:u16)->u8 {
    if !valid(w,h) {return 0;}picture_gate(addr_of_mut!(ACTIVE),address,w,h);
    ORIGIN_X=ACTIVE.sx;BIAS_X=ACTIVE.sx as u8;ORIGIN_Y=ACTIVE.sy;BIAS_Y=ACTIVE.sy as u8;
    pano_camera_x=ORIGIN_X+16;pano_camera_y=ORIGIN_Y+8;
    REQUESTED_X=0;REQUESTED_Y=0;DIRECTION_X=0;DIRECTION_Y=0;HORIZONTAL=0;
    PHASE=0;pano_prefetched=0;pano_retired=0;
    invalidate_bands();publish_camera();1
}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_init(address:u32,w:u16,h:u16)->u8 {
    if initialize(address,w,h)==0 {return 0;}reu_fetch(ACTIVE.reu,ram_ptr(CACHE),32768);1
}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_request(dx:i8,dy:i8) {
    if PHASE!=0 {return;}
    REQUESTED_X=if dx<0 {-1}else if dx>0 {1}else{0};REQUESTED_Y=if dy<0 {-1}else if dy>0 {1}else{0};
    if dx!=0 {DIRECTION_X=REQUESTED_X;}if dy!=0 {DIRECTION_Y=REQUESTED_Y;}
}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_tick() {
    if PHASE!=0 {
        if PHASE==2&&SLIDE<SLIDE_LENGTH {let n=SLIDE+1;let visible=n.wrapping_sub(SLIDE_LENGTH-200);
            if n<=SLIDE_LENGTH-200||(visible as u16+1)/2<=SEED_COLUMN as u16 {
                SLIDE=n;havoc_raster_x=START_X.wrapping_add(SLIDE);if SLIDE==SLIDE_LENGTH {PHASE=3;}}}
        return;
    }
    let x=moved(pano_camera_x,REQUESTED_X,ACTIVE.w-200);let y=moved(pano_camera_y,REQUESTED_Y,ACTIVE.h-240);
    let lower=ORIGIN_X+if HORIZONTAL>0 {16}else{0};let upper=ORIGIN_X+256-if HORIZONTAL<0 {16}else{0};
    if x>=lower&&x+200<=upper {pano_camera_x=x;REQUESTED_X=0;}
    if y>=ORIGIN_Y&&y+240<=ORIGIN_Y+256 {pano_camera_y=y;REQUESTED_Y=0;}
    publish_camera();
}
#[link_section=".highcode"] #[inline(never)] unsafe fn fetch_tile(x:u16,y:u16) {
    let index=(x>>4).wrapping_mul(ACTIVE.sh>>3).wrapping_add(y>>3);
    reu_fetch(ACTIVE.reu+32768+((index as u32)<<6),ram_ptr(TILE),64);
}
// 0: ordinary vertical lookahead;1: active strip;2: prepare vertical margin.
#[inline(always)] unsafe fn strip_gate()->u8 {
    #[cfg(test)] {start_strip()} #[cfg(not(test))] {pano_strip()}
}
#[cfg_attr(not(test),export_name="pano_strip_banked")]
#[cfg_attr(not(test),link_section=".bankcode")]
#[inline(never)] unsafe extern "C" fn start_strip()->u8 {
    if HORIZONTAL!=0 {return 1;}let offset=pano_camera_x-ORIGIN_X;
    // Start at the first safe point: the outgoing16-pixel strip is outside
    // the200-pixel viewport. This leaves40 pixels of travel before a stall.
    let direction=if DIRECTION_X>0&&offset>=16&&ORIGIN_X+256<((ACTIVE.w+15)&65520) {1}
        else if DIRECTION_X<0&&offset<=40&&ORIGIN_X!=0 {-1}else{return 0;};
    // A strip freezes ORIGIN_Y until all256 rows are loaded. First give the
    // camera12 rows of runway, without chasing it through another whole band.
    if (DIRECTION_Y>0&&pano_camera_y-ORIGIN_Y>4&&ORIGIN_Y+256<ACTIVE.sh)||
       (DIRECTION_Y<0&&pano_camera_y-ORIGIN_Y<12&&ORIGIN_Y!=0) {return 2;}
    HORIZONTAL=direction;STRIP_X=if direction>0 {ORIGIN_X+256}else{ORIGIN_X-16};
    STRIP_Y=ORIGIN_Y&65528;STRIP_TILE=0;STRIP_TILES=(((ORIGIN_Y&7)+263)>>3) as u8;
    1
}
#[link_section=".highcode"] #[inline(never)] unsafe fn strip_job() {
    let y=STRIP_Y+((STRIP_TILE as u16)<<3);fetch_tile(STRIP_X,y);
    let first=if y<ORIGIN_Y {(ORIGIN_Y-y) as u8}else{0};
    let end=core::cmp::min(ORIGIN_Y+256-y,8) as u8;let count=end-first;
    let physical=(y as u8).wrapping_add(first).wrapping_sub(BIAS_Y);
    let packed=(STRIP_X as u8).wrapping_sub(BIAS_X)/2;
    #[cfg(not(test))] {
        pano_copy_strip(ram_ptr(CACHE+((packed as u16)<<8)+physical as u16),
            ram_ptr(TILE+first as u16),count as u16);
    }
    #[cfg(test)] {
        let contiguous=core::cmp::min(count as u16,256-physical as u16) as usize;
        let mut source=ram_ptr(TILE+first as u16);
        let mut c=0u8;while c<8 {
            let column=packed.wrapping_add(c)&127;
            let dest=ram_ptr(CACHE+((column as u16)<<8));
            core::ptr::copy_nonoverlapping(source,dest.add(physical as usize),contiguous);
            if contiguous<count as usize {core::ptr::copy_nonoverlapping(source.add(contiguous),dest,count as usize-contiguous);}
            source=source.add(8);c+=1;
        }
    }
    STRIP_TILE+=1;if STRIP_TILE==STRIP_TILES {ORIGIN_X=if HORIZONTAL>0 {ORIGIN_X+16}else{ORIGIN_X-16};HORIZONTAL=0;retain_gate();}
}
#[link_section=".highcode"] #[inline(never)] unsafe fn get_band(y:u16,avoid:u8)->u8 {
    let mut i=0;while i<2 {if BAND_Y[i as usize]==y {return i;}i+=1;}
    let i=if avoid==0 {1}else{0};BAND_Y[i as usize]=y;BAND_COLUMNS[i as usize]=0;i
}
#[inline(never)] unsafe fn copy_band_tile(b:u8,x:u16) {
    fetch_tile(x,BAND_Y[b as usize]);
    let packed=(x as u8).wrapping_sub(BIAS_X)/2;let base=ram_ptr(BANDS+((b as u16)<<10));
    #[cfg(not(test))] {pano_copy_band(base.add(packed as usize),ram_ptr(TILE),64);}
    #[cfg(test)] {
        let mut source=ram_ptr(TILE);let mut c=0u8;
        while c<8 {let mut dest=base.add((packed.wrapping_add(c)&127) as usize);let mut r=8u8;
            while r!=0 {dest.write(source.read());source=source.add(1);dest=dest.add(128);r-=1;}
            c+=1;
        }
    }
}
#[link_section=".highcode"] #[inline(never)] unsafe fn band_job(b:u8) {
    let column=BAND_COLUMNS[b as usize];copy_band_tile(b,ORIGIN_X+((column as u16)<<4));BAND_COLUMNS[b as usize]+=1;
}
#[link_section=".highcode"] #[inline(never)] unsafe fn row_job(b:u8,row:u16,d:i8) {
    let y=(row as u8).wrapping_sub(BIAS_Y) as u16;
    let source=ram_ptr(BANDS+((b as u16)<<10)+((row&7)<<7));let dest=ram_ptr(CACHE+y);
    #[cfg(not(test))] {pano_copy_row(dest,source,128);}
    #[cfg(test)] {
        let mut source=source;let mut dest=dest;let mut count=128u8;
        while count!=0 {dest.write(source.read());source=source.add(1);dest=dest.add(256);count-=1;}
    }
    ORIGIN_Y=if d>0 {ORIGIN_Y+1}else{ORIGIN_Y-1};
}
// Guards cover the job and the outer loop return. The 1430-tick deadline
// already leaves about 65 PHI2 ticks before the next raster top (~1495).
#[inline(always)] unsafe fn available(deadline:u32,cost:i32)->bool {timer_read().wrapping_sub(deadline) as i32>cost+16}
#[link_section=".highcode"] #[inline(never)] unsafe fn normal_job(deadline:u32)->u8 {
    // Repairs precede both a new strip (which replaces STRIP_X) and any row
    // copy (which must see a band matching the current horizontal origin).
    if BAND_REPAIR!=0 {
        if !available(deadline,104) {return 0;}
        let b=if BAND_REPAIR&1!=0 {0}else{1};copy_band_tile(b,STRIP_X);BAND_REPAIR&=!(1<<b);return 1;
    }
    let mode=strip_gate();if mode==1 {if !available(deadline,108) {return 0;}strip_job();return 1;}
    let d=DIRECTION_Y;
    let row=if d>0&&ORIGIN_Y+256<ACTIVE.sh {ORIGIN_Y+256}else if d<0&&ORIGIN_Y!=0 {ORIGIN_Y-1}else{return 0;};
    let band=row&65528;let b=get_band(band,255);
    if BAND_COLUMNS[b as usize]<16 {if !available(deadline,104) {return 0;}band_job(b);return 1;}
    let target=if mode==2 {if d>0 {4}else{12}}else{8};
    if (d>0&&pano_camera_y-ORIGIN_Y>target)||(d<0&&pano_camera_y-ORIGIN_Y<target) {if !available(deadline,56) {return 0;}row_job(b,row,d);return 1;}
    let next=if d>0 {let n=band+8;if n>=ACTIVE.sh {return 0;}n}else{if band==0 {return 0;}band-8};
    let other=get_band(next,b);if BAND_COLUMNS[other as usize]<16 {if !available(deadline,104) {return 0;}band_job(other);return 1;}0
}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_prefetch(address:u32,w:u16,h:u16)->u8 {
    if PHASE!=0||!valid(w,h) {return 0;}picture_gate(addr_of_mut!(NEXT),address,w,h);PHASE=1;HORIZONTAL=0;
    REQUESTED_X=0;REQUESTED_Y=0;pano_prefetched=0;pano_retired=0;
    START_X=havoc_raster_x;START_Y=havoc_raster_y;SLIDE=0;SLIDE_LENGTH=200+(START_X&1);
    SEAM=START_X.wrapping_add(SLIDE_LENGTH);SEED_COLUMN=0;SEED_PART=0;1
}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_can_transition()->u8 {(pano_prefetched!=0&&PHASE==1) as u8}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_transition() {if pano_can_transition()!=0 {PHASE=2;}}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_transition_done()->u8 {pano_retired}
#[no_mangle] #[link_section=".highcode"] #[inline(never)] pub unsafe extern "C" fn pano_busy()->u8 {(PHASE!=0) as u8}
/// Farthest camera positions for the active picture (200x240 logical viewport).
#[inline(always)] pub unsafe fn pano_limit_x()->u16 {ACTIVE.w.wrapping_sub(200)}
#[inline(always)] pub unsafe fn pano_limit_y()->u16 {ACTIVE.h.wrapping_sub(240)}
#[link_section=".highcode"] #[inline(never)] unsafe fn seed_job()->u8 {
    if SEED_COLUMN==128 {return 0;}let relative=SEAM.wrapping_add(SEED_COLUMN.wrapping_mul(2)).wrapping_sub(START_X);
    if (relative<SLIDE_LENGTH&&relative>=SLIDE)||(relative.wrapping_add(1)<SLIDE_LENGTH&&relative.wrapping_add(1)>=SLIDE) {return 0;}
    let physical=START_Y.wrapping_add(SEED_PART*64) as u16;let first=core::cmp::min(256-physical,64);
    let source=NEXT.reu+((SEED_COLUMN as u32)<<8)+((SEED_PART as u32)<<6);
    let dest=CACHE+((((SEAM/2).wrapping_add(SEED_COLUMN)&127) as u16)<<8);
    reu_fetch(source,ram_ptr(dest+physical),first);if first<64 {reu_fetch(source+first as u32,ram_ptr(dest),64-first);}
    SEED_PART+=1;if SEED_PART==4 {SEED_PART=0;SEED_COLUMN+=1;}if SEED_COLUMN>=8 {pano_prefetched=1;}
    if SEED_COLUMN==128&&PHASE==3 {
        ACTIVE=NEXT;ORIGIN_X=ACTIVE.sx;ORIGIN_Y=ACTIVE.sy;
        BIAS_X=(ACTIVE.sx as u8).wrapping_sub(SEAM);BIAS_Y=(ACTIVE.sy as u8).wrapping_sub(START_Y);
        pano_camera_x=ACTIVE.sx;pano_camera_y=ACTIVE.sy;invalidate_bands();DIRECTION_X=0;DIRECTION_Y=0;
        PHASE=0;pano_retired=1;publish_camera();
    }1
}
#[no_mangle] #[inline(never)] pub unsafe extern "C" fn pano_step(jobs:u8,deadline:u32)->u8 {
    let mut done=0;while done<jobs {let begin=timer_read();
        let worked=if PHASE!=0 {if !available(deadline,128) {return done;}seed_job()}else{normal_job(deadline)};if worked==0 {return done;}
        let elapsed=begin.wrapping_sub(timer_read()) as u16;if elapsed>pano_max_job_ticks {pano_max_job_ticks=elapsed;}done+=1;
    }done
}
#[cfg(test)] pub mod test_access {
    use super::*;
    pub unsafe fn bias()->(u8,u8) {(BIAS_X,BIAS_Y)}
    pub unsafe fn origin()->(u16,u16) {(ORIGIN_X,ORIGIN_Y)}
}
