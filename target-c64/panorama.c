#include "include/panorama.h"
#include "include/platform.h"
#include "include/raster.h"

#define HIGH __attribute__((section(".highcode"),noinline))
#ifdef HAVOC_PANORAMA_TEST
extern uint8_t pano_test_ram[65536];
#define RAM(a) (pano_test_ram+(a))
#else
#define RAM(a) ((uint8_t *)(a))
#endif
#define TILEBUF RAM(0xe6c0u)
#define BAND(n) RAM(0xe700u+((uint16_t)(n)<<10))
#define INVALID 65535u

typedef struct { uint32_t reu; uint16_t w,h,sh,sx,sy; } Picture;
static Picture active,next;
static uint16_t origin_x,origin_y;
static uint8_t bias_x,bias_y;
static uint16_t band_y[2];
static uint8_t band_columns[2];
static int8_t requested_x,requested_y,direction_x,direction_y;
static int8_t horizontal;
static uint16_t strip_x,strip_y;
static uint8_t strip_tile,strip_tiles;
static uint8_t margin;
/* phase 0 normal, 1 hidden prefetch, 2 slide, 3 finish hidden tail. */
static uint8_t phase,seed_column,seed_part,start_x,start_y,seam;
static uint8_t slide,slide_length;
volatile uint16_t pano_camera_x,pano_camera_y;
volatile uint16_t pano_max_job_ticks=256;
volatile uint8_t pano_prefetched,pano_retired;

static HIGH uint16_t storage_width(void) { return (active.w+15u)&65520u; }
static HIGH uint16_t max_x(void) { return active.w-200u; }
static HIGH uint16_t max_y(void) { return active.h-240u; }
static HIGH uint8_t valid(uint16_t w,uint16_t h) {
    /* The production catalog is 905x1086. Small pictures retain their seeded
     * clamped image but do not enter the scrolling path. */
    return w>=256 && h>=256 && w<32752 && h<32760;
}
static HIGH void picture(Picture *p,uint32_t address,uint16_t w,uint16_t h) {
    p->reu=address;p->w=w;p->h=h;p->sh=(h+7u)&65528u;
    p->sx=((w-256u)/2u)&65520u;p->sy=((h-256u)/2u)&65528u;
}
static HIGH void invalidate_bands(void) {
    band_y[0]=band_y[1]=INVALID;band_columns[0]=band_columns[1]=0;
}
static HIGH void publish_camera(void) {
    havoc_raster_x=(uint8_t)(pano_camera_x-bias_x);
    havoc_raster_y=(uint8_t)(pano_camera_y-bias_y);
}
/* This startup helper returns BEFORE the resident wrapper overwrites cache
 * RAM. An overlay must never DMA over its own return path. */
__attribute__((section(".disk_init"),noinline))
static uint8_t initialize(uint32_t address,uint16_t w,uint16_t h) {
    if(!valid(w,h))return 0;
    picture(&active,address,w,h);
    origin_x=active.sx;bias_x=(uint8_t)active.sx;
    origin_y=active.sy;bias_y=(uint8_t)active.sy;
    pano_camera_x=origin_x+16;pano_camera_y=origin_y+8;
    requested_x=requested_y=direction_x=direction_y=horizontal=0;
    phase=margin=0;pano_prefetched=pano_retired=0;invalidate_bands();publish_camera();
    return 1;
}
__attribute__((section(".midcode"),noinline)) uint8_t pano_init(uint32_t address,uint16_t w,uint16_t h) {
    if(!initialize(address,w,h))return 0;
    reu_fetch(active.reu,RAM(HAVOC_CACHE_ADDRESS),32768);
    return 1;
}
HIGH void pano_request(int8_t dx,int8_t dy) {
    if(phase)return;
    requested_x=dx<0?-1:dx>0?1:0;requested_y=dy<0?-1:dy>0?1:0;
    if(dx)direction_x=requested_x;
    if(dy)direction_y=requested_y;
}
static HIGH uint16_t moved(uint16_t at,int8_t d,uint16_t maximum) {
    if(d<0)return at?at-1:0;
    if(d>0)return at<maximum?at+1:maximum;
    return at;
}
HIGH void pano_tick(void) {
    uint16_t x,y,lower,upper;
    if(phase) {
        if(phase==2 && slide<slide_length) {
            uint8_t n=slide+1,visible=n-(slide_length-200u);
            if(n<=slide_length-200u || (visible+1u)/2u<=seed_column) {
                slide=n;havoc_raster_x=(uint8_t)(start_x+slide);
                if(slide==slide_length)phase=3;
            }
        }
        return;
    }
    x=moved(pano_camera_x,requested_x,max_x());
    y=moved(pano_camera_y,requested_y,max_y());
    lower=origin_x+(horizontal>0?16u:0u);
    upper=origin_x+256u-(horizontal<0?16u:0u);
    if(x>=lower+margin && x+200u+margin<=upper) {pano_camera_x=x;requested_x=0;}
    if(y>=origin_y+margin && y+240u+margin<=origin_y+256u) {pano_camera_y=y;requested_y=0;}
    publish_camera();
}
static HIGH void fetch_tile(uint16_t x,uint16_t y) {
    /* Validated panorama assets are capped at 3 MiB: fewer than 49,152 tiles,
     * so their tile index fits 16 bits. Keep only the REU address 32-bit. */
    uint16_t index=(x>>4)*(active.sh>>3)+(y>>3);
    reu_fetch(active.reu+32768UL+((uint32_t)index<<6),TILEBUF,64);
}
static HIGH void start_strip(void) {
    uint16_t offset=pano_camera_x-origin_x;
    if(horizontal)return;
    if(direction_x>0 && offset>=32u && origin_x+256u<storage_width()) {
        horizontal=1;strip_x=origin_x+256u;
    } else if(direction_x<0 && offset<=16u && origin_x) {
        horizontal=-1;strip_x=origin_x-16u;
    } else return;
    strip_y=origin_y&65528u;strip_tile=0;
    strip_tiles=(uint8_t)(((origin_y&7u)+263u)>>3);
}
static HIGH void strip_job(void) {
    uint8_t c,r;uint16_t y=strip_y+((uint16_t)strip_tile<<3);
    fetch_tile(strip_x,y);
    for(c=0;c<8;++c) {
        uint8_t column=((uint8_t)(strip_x-bias_x)/2u+c)&127u;
        uint8_t *dst=RAM(HAVOC_CACHE_ADDRESS+((uint16_t)column<<8));
        for(r=0;r<8;++r)if(y+r>=origin_y && y+r<origin_y+256u)
            dst[(uint8_t)(y+r-bias_y)]=TILEBUF[c*8u+r];
    }
    if(++strip_tile==strip_tiles) {
        origin_x=horizontal>0?origin_x+16u:origin_x-16u;
        horizontal=0;invalidate_bands();
    }
}
static HIGH uint8_t get_band(uint16_t y,uint8_t avoid) {
    uint8_t i;
    /* Horizontal commits invalidate both bands, so their X origin is always
     * the current cache origin and does not need separate stored tags. */
    for(i=0;i<2;++i)if(band_y[i]==y)return i;
    i=avoid==0?1:0;
    band_y[i]=y;band_columns[i]=0;return i;
}
static HIGH void band_job(uint8_t b) {
    uint8_t c,r,column=band_columns[b];
    uint16_t x=origin_x+((uint16_t)column<<4);
    uint8_t packed=(uint8_t)(x-bias_x)/2u;
    fetch_tile(x,band_y[b]);
    for(r=0;r<8;++r)for(c=0;c<8;++c)
        BAND(b)[((uint16_t)r<<7)+(uint8_t)((packed+c)&127u)]=TILEBUF[c*8u+r];
    ++band_columns[b];
}
static HIGH void row_job(uint8_t b,uint16_t row,int8_t d) {
    uint8_t c,y=(uint8_t)(row-bias_y);
    uint8_t *src=BAND(b)+((row&7u)<<7);
    for(c=0;c<128;++c)RAM(HAVOC_CACHE_ADDRESS+((uint16_t)c<<8))[y]=src[c];
    if(d>0)++origin_y;else --origin_y;
}
static HIGH uint8_t normal_job(void) {
    uint16_t row,band,next_band;uint8_t b,other;
    int8_t d=direction_y;
    start_strip();
    if(horizontal) {strip_job();return 1;}
    if(d>0 && origin_y+256u<active.sh)row=origin_y+256u;
    else if(d<0 && origin_y)row=origin_y-1u;
    else return 0;
    band=row&65528u;b=get_band(band,255);
    if(band_columns[b]<16) {band_job(b);return 1;}
    if((d>0 && (uint16_t)(pano_camera_y-origin_y)>8u) ||
       (d<0 && (uint16_t)(pano_camera_y-origin_y)<8u)) {row_job(b,row,d);return 1;}
    if(d>0) {next_band=band+8u;if(next_band>=active.sh)return 0;}
    else {if(!band)return 0;next_band=band-8u;}
    other=get_band(next_band,b);
    if(band_columns[other]<16) {band_job(other);return 1;}
    return 0;
}
HIGH uint8_t pano_prefetch(uint32_t address,uint16_t w,uint16_t h) {
    if(phase || margin || !valid(w,h))return 0;
    picture(&next,address,w,h);phase=1;horizontal=0;
    requested_x=requested_y=0;pano_prefetched=pano_retired=0;
    start_x=havoc_raster_x;start_y=havoc_raster_y;
    slide=0;slide_length=200u+(start_x&1u);
    seam=(uint8_t)(start_x+slide_length);seed_column=seed_part=0;
    return 1;
}
HIGH uint8_t pano_can_transition(void) {return pano_prefetched && phase==1;}
HIGH void pano_transition(void) {if(pano_can_transition())phase=2;}
HIGH uint8_t pano_transition_done(void) {return pano_retired;}
HIGH uint8_t pano_busy(void) {return phase!=0;}
HIGH uint8_t pano_tilt_guard(uint8_t enabled) {
    uint16_t left,right;
    if(!enabled) {margin=0;return 1;}
    if(phase)return 0;
    left=origin_x+(horizontal>0?16u:0u)+4u;
    right=origin_x+256u-(horizontal<0?16u:0u);
    if(pano_camera_x<left || pano_camera_x+204u>right ||
       pano_camera_y<origin_y+4u || pano_camera_y+244u>origin_y+256u)return 0;
    margin=4;return 1;
}
static HIGH uint8_t seed_job(void) {
    uint8_t relative=(uint8_t)(seam+seed_column*2u-start_x);
    uint16_t physical,first;uint32_t source;
    if(seed_column==128)return 0;
    /* Columns are only written after leaving the old viewport, or before
     * entering from its right. A complete column is published atomically. */
    if((relative<slide_length && relative>=slide) ||
       ((uint8_t)(relative+1u)<slide_length && (uint8_t)(relative+1u)>=slide))return 0;
    physical=(uint8_t)(start_y+seed_part*64u);
    first=256u-physical;if(first>64u)first=64u;
    source=next.reu+((uint32_t)seed_column<<8)+((uint16_t)seed_part<<6);
    {
        uint16_t dest=HAVOC_CACHE_ADDRESS+((uint16_t)((seam/2u+seed_column)&127u)<<8);
        reu_fetch(source,RAM(dest+physical),first);
        if(first<64u)reu_fetch(source+first,RAM(dest),64u-first);
    }
    if(++seed_part==4) {seed_part=0;++seed_column;}
    if(seed_column>=8)pano_prefetched=1;
    if(seed_column==128 && phase==3) {
        active=next;origin_x=active.sx;origin_y=active.sy;
        bias_x=(uint8_t)(active.sx-seam);bias_y=(uint8_t)(active.sy-start_y);
        pano_camera_x=active.sx;pano_camera_y=active.sy;
        invalidate_bands();direction_x=direction_y=0;
        phase=0;pano_retired=1;publish_camera();
    }
    return 1;
}
__attribute__((noinline)) uint8_t pano_step(uint8_t jobs,uint32_t deadline) {
    uint8_t done=0;
    while(done<jobs) {
        uint32_t begin=timer_read();uint16_t elapsed;
        uint8_t worked;
        if((int32_t)(begin-deadline)<=(int32_t)pano_max_job_ticks+32)return done;
        worked=phase?seed_job():normal_job();
        if(!worked)return done;
        elapsed=(uint16_t)(begin-timer_read());
        if(elapsed>pano_max_job_ticks)pano_max_job_ticks=elapsed;
        ++done;
    }
    return done;
}
