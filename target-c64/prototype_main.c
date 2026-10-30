/* First hardware gate: compressed real-time stereo + border raster + UCI reads.
 * No waveform is predecoded in the package. Production playlist follows the gate. */
#include <stdint.h>
#include "include/platform.h"
#include "include/raster.h"
#include "include/audio_step.h"
#include "include/prototype_assets.h"

volatile uint8_t havoc_audio_chunk_state[6],havoc_audio_chunk_units[6],havoc_audio_chunk_reset[6];
uint8_t havoc_audio_codes_hi[6],havoc_audio_pcm_hi[6],havoc_audio_lut_hi[6];
static const uint8_t chunk_page[6]={0xc0,0xc3,0xc6,0xc9,0xe0,0xe3};
/* A non-FREE chunk pins its shared lookup page, including DONE chunks whose
 * PCM has not reached the REU yet. Reuse book contents across loop epochs: the
 * recording's codebooks are immutable, while RESET stays a per-chunk flag. */
static uint16_t chunk_book[6];
static uint8_t chunk_lut[6];
#define INPUT ((uint8_t *)0xe600)
#define BOOK ((uint8_t *)0xe680)
#define SCRATCH ((uint8_t *)0xe6a0)

/* Public symbols are inspectable in the supplied ELF; display snapshot via F1. */
volatile uint16_t havoc_underruns,havoc_late_frames,havoc_late_lines_total;
volatile uint16_t havoc_dma_ticks,havoc_minimum_lead=32767;
volatile uint8_t havoc_fault;
static uint8_t uci_error,uci_enabled;
static uint16_t uci_reads;
static uint8_t fill_index,drain_index,reset_next=1,running,paused,automatic=1,overlay;
static uint8_t old_keys=255,old_plus=255,frame_divider,grid,text_row=2;
static int8_t dx=1,dy=1;
static uint16_t segment,segment_left=AUDIO_SHORT_SEGMENT+(AUDIO_LONG_SEGMENTS?4:0),divider;
static uint32_t source_frame,stream_position,produced,consumed,last_tick,clock_remainder;

static void audio_init(void) {
    uint8_t i;
    for(i=0;i<6;++i) {
        havoc_audio_chunk_state[i]=0;
        havoc_audio_codes_hi[i]=chunk_page[i];havoc_audio_pcm_hi[i]=chunk_page[i]+1;
        havoc_audio_lut_hi[i]=chunk_page[i]+2;
    }
    for(i=0;i<4;++i)havoc_audio_predictors[i]=0;
    havoc_audio_position=0;havoc_audio_quota=0;havoc_audio_current=5;
}
static void prepare_chunk(uint8_t slot) {
    uint8_t i,n,frames,code_index,table=255,used=0;
    uint8_t *codes=(uint8_t *)((uint16_t)havoc_audio_codes_hi[slot]<<8);
    uint8_t *lut;
    uint16_t left=segment_left;
    if(source_frame==AUDIO_FRAMES) {
        source_frame=stream_position=0;segment=0;reset_next=1;
        segment_left=AUDIO_SHORT_SEGMENT+(AUDIO_LONG_SEGMENTS?4:0);left=segment_left;
    }
    if(!left) {
        ++segment; segment_left=AUDIO_SHORT_SEGMENT+(segment<AUDIO_LONG_SEGMENTS?4:0);left=segment_left;
    }
    frames=left>128?128:(uint8_t)left;
    if(AUDIO_FRAMES-source_frame<frames)frames=(uint8_t)(AUDIO_FRAMES-source_frame);
    /* Diagnostic resampling produces a whole multiple of four stereo frames. */
    frames &= 252;
    if(!frames) { havoc_fault=4;return; }
    n=(frames>>2)*3;
    for(i=0;i<6;++i)if(havoc_audio_chunk_state[i]!=HAVOC_AUDIO_FREE) {
        if(chunk_book[i]==segment) {table=chunk_lut[i];break;}
        used|=(uint8_t)(1u<<chunk_lut[i]);
    }
    if(table==255) {
        /* This slot is FREE, so at most five different pages can be pinned. */
        table=0;while(used&1) {used>>=1;++table;}
        lut=(uint8_t *)((uint16_t)(chunk_page[table]+2)<<8);
        reu_fetch(AUDIO_REU+32+(uint32_t)segment*32,BOOK,32);
        for(i=0;i<64;++i) {
            uint8_t l=(i&7)*2,r=16+(i>>3)*2;
            lut[i]=BOOK[l];lut[i+64]=BOOK[l+1];lut[i+128]=BOOK[r];lut[i+192]=BOOK[r+1];
        }
    }
    chunk_book[slot]=segment;chunk_lut[slot]=table;
    havoc_audio_lut_hi[slot]=chunk_page[table]+2;
    reu_fetch(AUDIO_REU+AUDIO_STREAM_OFFSET+stream_position,INPUT,n);
    code_index=0;
    for(i=0;i<n;i+=3) {
        uint8_t a=INPUT[i],b=INPUT[i+1],c=INPUT[i+2];
        codes[code_index]=a&63;codes[(uint8_t)(code_index+2)]=((a>>6)|(b<<2))&63;
        codes[(uint8_t)(code_index+4)]=((b>>4)|(c<<4))&63;codes[(uint8_t)(code_index+6)]=c>>2;
        code_index+=8;
    }
    source_frame+=frames;stream_position+=n;segment_left-=frames;
    havoc_audio_chunk_units[slot]=frames>>1;havoc_audio_chunk_reset[slot]=reset_next;reset_next=0;
    /* Publish after all bytes are committed; raster is not interrupt-driven. */
    havoc_audio_chunk_state[slot]=HAVOC_AUDIO_READY;
}
static void update_clock(void) {
    uint32_t now=timer_read(),elapsed=last_tick-now,scaled;
    last_tick=now;
    if(!running)return;
    /* A long service stall fails this timing gate. Never hide it by clamping
     * elapsed time and pretending the sampler consumed fewer frames. */
    if(elapsed>65535UL) { ++havoc_late_frames; havoc_fault=6; sampler_stop(); running=0; return; }
    scaled=elapsed*32768UL+clock_remainder;
    consumed+=scaled/sampler_ticks_per_ring;clock_remainder=scaled%sampler_ticks_per_ring;
    if(produced<=consumed+256) {
        ++havoc_underruns;havoc_fault=5;sampler_stop();running=0;return;
    }
    if(produced-consumed<havoc_minimum_lead)havoc_minimum_lead=(uint16_t)(produced-consumed);
    if(consumed>=0x01000000UL) {consumed-=0x01000000UL;produced-=0x01000000UL;}
}
static void service_audio(void) {
    uint8_t count=0;
    uint32_t begin=timer_read();
    while(count++<6 && havoc_audio_chunk_state[drain_index]==HAVOC_AUDIO_DONE) {
        uint16_t bytes=(uint16_t)havoc_audio_chunk_units[drain_index]*4;
        uint16_t at=(uint16_t)(produced*2);
        uint8_t *pcm=(uint8_t *)((uint16_t)havoc_audio_pcm_hi[drain_index]<<8);
        uint16_t first=bytes;
        if(at && (uint32_t)at+bytes>65536UL)first=(uint16_t)(65536UL-at);
        if(produced-consumed+bytes/2>=32768UL)break;
        reu_stash(pcm,RING_ADDRESS+at,first);
        if(first<bytes)reu_stash(pcm+first,RING_ADDRESS,bytes-first);
        produced+=bytes/2;havoc_audio_chunk_state[drain_index]=HAVOC_AUDIO_FREE;
        if(++drain_index==6)drain_index=0;
    }
    count=0;
    while(count++<6 && havoc_audio_chunk_state[fill_index]==HAVOC_AUDIO_FREE) {
        prepare_chunk(fill_index);if(havoc_fault)return;
        if(++fill_index==6)fill_index=0;
    }
    havoc_dma_ticks=(uint16_t)(begin-timer_read());
}
static uint8_t keyrow(uint8_t mask) {
    REG(0xdc02)=255;REG(0xdc03)=0;REG(0xdc00)=mask;return REG(0xdc01);
}
static void pixel(uint8_t x,uint8_t y,uint8_t color) {
    volatile uint8_t *p=HAVOC_RASTER_CACHE+((uint16_t)(x>>1)<<8)+y;
    *p=x&1?((*p&15)|(color<<4)):((*p&240)|color);
}
static const uint8_t digits[16][5]={
 {31,17,17,17,31},{4,12,4,4,14},{31,1,31,16,31},{31,1,15,1,31},
 {17,17,31,1,1},{31,16,31,1,31},{31,16,31,17,31},{31,1,2,4,4},
 {31,17,31,17,31},{31,17,31,1,31},{14,17,31,17,17},{30,17,30,17,30},
 {31,16,16,16,31},{30,17,17,17,30},{31,16,30,16,31},{31,16,30,16,16}};
__attribute__((section(".lowcode"),noinline)) static void hexword(uint8_t x,uint16_t value) {
    uint8_t d,y,b;
    for(d=0;d<4;++d) {uint8_t n=value>>12;value<<=4;
        for(y=0;y<5;++y)for(b=0;b<5;++b)pixel(x+d*6+b,text_row+y,(digits[n][y]&(16>>b))?1:0);
    }
}
static void snapshot(void) {
    havoc_raster_x=havoc_raster_y=0;
    text_row=2;
    hexword(2,havoc_late_lines_total);hexword(28,havoc_underruns);hexword(54,havoc_late_frames);
    hexword(80,uci_reads);hexword(106,((uint16_t)havoc_fault<<8)|uci_error);
    hexword(132,havoc_minimum_lead);hexword(158,havoc_dma_ticks);
    text_row=10;hexword(2,havoc_dma8_budget_ticks);hexword(28,havoc_dma16_budget_ticks);
    hexword(54,havoc_raster_top);hexword(80,havoc_raster_delay);hexword(106,divider);
}
static void controls(void) {
    uint8_t keys=keyrow(254),pressed=old_keys&~keys,plus=keyrow(223),joystick;
    old_keys=keys;
    if(pressed&16) {overlay=!overlay;automatic=!overlay;if(!overlay && !grid)reu_fetch(0x10000UL,(void*)HAVOC_CACHE_ADDRESS,32768);}
    if(pressed&32)uci_enabled=!uci_enabled;
    if(pressed&64) {grid=!grid;if(grid)havoc_raster_init();else reu_fetch(0x10000UL,(void*)HAVOC_CACHE_ADDRESS,32768);}
    if(pressed&8) {paused=!paused;REG(0xdf21)=paused?0:32;REG(0xdf41)=paused?0:32;}
    /* Calibration +/-; shift selects vertical start instead of horizontal delay. */
    {uint8_t shift=!(keyrow(253)&128) || !(keyrow(191)&16);
     if((old_plus&~plus)&1) {if(shift){if(havoc_raster_top<63)++havoc_raster_top;}else if(havoc_raster_delay<24)++havoc_raster_delay;}
     if((old_plus&~plus)&8) {if(shift){if(havoc_raster_top)--havoc_raster_top;}else if(havoc_raster_delay>1)--havoc_raster_delay;}}
    old_plus=plus;REG(0xdc02)=0;REG(0xdc00)=255;joystick=REG(0xdc00);
    if((joystick&15)!=15) {automatic=0;overlay=0;
        if(!(joystick&1)&&havoc_raster_y)--havoc_raster_y;
        if(!(joystick&2)&&havoc_raster_y<16)++havoc_raster_y;
        if(!(joystick&4)&&havoc_raster_x)--havoc_raster_x;
        if(!(joystick&8)&&havoc_raster_x<56)++havoc_raster_x;}
    if(!(joystick&16)) {automatic=1;overlay=0;}
    if(automatic && !(++frame_divider&1)) {
        if((havoc_raster_x==56&&dx>0)||(!havoc_raster_x&&dx<0))dx=-dx;
        if((havoc_raster_y==16&&dy>0)||(!havoc_raster_y&&dy<0))dy=-dy;
        havoc_raster_x+=dx;havoc_raster_y+=dy;
    }
    if(overlay)snapshot();
}
int main(void) {
    uint16_t n;uint8_t late;
    platform_init();REG(0xd020)=14;audio_init();havoc_raster_init();
    /* The prototype is loaded with its configuration's matching REU fixture. */
    reu_fetch(0,SCRATCH,16);
    if(SCRATCH[0]!='H'||SCRATCH[1]!='V'||SCRATCH[7]!='1')havoc_fault=1;
    if(!havoc_fault && SCRATCH[8])reu_fetch(0x10000UL,(void*)HAVOC_CACHE_ADDRESS,32768);else grid=1;
    /* Clear the sampler ring before the silent rate probe. */
    for(n=0;n<256;++n)INPUT[n]=0;
    probe_dma();
    if(!havoc_fault)for(n=0;n<256;++n)reu_stash(INPUT,RING_ADDRESS+((uint32_t)n<<8),256);
    if(!havoc_fault && REG(0xdf21)!=0x10)havoc_fault=2;
    if(!havoc_fault) {REG(0xd020)=7;divider=sampler_calibrate();if(!divider)havoc_fault=3;}
    /* Solid failure colors need neither the turbo renderer nor a valid cache.
     * Missing REU: red; missing sampler: purple; calibration failure: orange. */
    if(havoc_fault) {
        REG(0xd020)=havoc_fault==1?2:havoc_fault==2?4:8;
        for(;;) {}
    }
    REG(0xd020)=5;
    if(!havoc_fault) {
        while(produced<24576UL) {
            service_audio();for(n=0;n<384;++n)havoc_raster_work();
            havoc_raster_frame();if(havoc_fault)break;
        }
        last_tick=timer_read();sampler_start(divider);running=1;
    }
    late=havoc_raster_late_lines;
    for(;;) {
        uint32_t before=timer_read();
        havoc_raster_frame();
        havoc_late_lines_total+=(uint8_t)(havoc_raster_late_lines-late);late=havoc_raster_late_lines;
        update_clock();
        if(!havoc_fault) {
            service_audio();
            /* Remaining per-frame decode work after the 240 scanline slots. */
            for(n=0;n<40;++n)havoc_raster_work();
            service_audio();
        }
        controls();
        late=havoc_raster_late_lines;
        if(before-timer_read()>26000UL)++havoc_late_frames;
    }
}
