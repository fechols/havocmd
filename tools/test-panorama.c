/* Native production-cache tests; no hardware timing is claimed.
 * gcc -O2 -Wall -Wextra tools/test-panorama.c -o /tmp/test-panorama */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#define HAVOC_PANORAMA_TEST 1
unsigned char pano_test_ram[65536];
#include "../target-c64/panorama.c"
volatile uint8_t havoc_raster_x,havoc_raster_y,havoc_raster_top,havoc_raster_delay;
volatile uint8_t havoc_raster_late_lines,havoc_raster_frames;
static uint8_t reu_memory[2*1024*1024];
static uint32_t ticks=0xf0000000u;
static unsigned transfers;
uint32_t timer_read(void) { return --ticks; }
void reu_fetch(uint32_t source,void *dest,uint16_t n) {
    assert(source+n<=sizeof(reu_memory));
    assert((uint8_t *)dest>=pano_test_ram && (uint8_t *)dest+n<=pano_test_ram+65536);
    memcpy(dest,reu_memory+source,n);ticks-=n+12;++transfers;
}
static uint8_t pixel(unsigned pic,unsigned x,unsigned y) {
    unsigned w=pic==2?905u:640u,h=pic==2?1086u:768u;
    if(x>=w)x=w-1;
    if(y>=h)y=h-1;
    return (uint8_t)(((x*13u)^(y*7u)^(x>>4)^(y>>6)^(pic*11u))&15u);
}
static void asset(uint32_t address,unsigned pic,unsigned w,unsigned h) {
    unsigned sx=((w-256)/2)&~15u,sy=((h-256)/2)&~7u;
    unsigned x,y,tx,ty;uint32_t at=address;
    for(x=0;x<256;x+=2)for(y=0;y<256;++y)
        reu_memory[at++]=pixel(pic,sx+x,sy+y)|(pixel(pic,sx+x+1,sy+y)<<4);
    for(tx=0;tx<((w+15u)&~15u);tx+=16)for(ty=0;ty<((h+7u)&~7u);ty+=8)
        for(x=0;x<16;x+=2)for(y=0;y<8;++y)
            reu_memory[at++]=pixel(pic,tx+x,ty+y)|(pixel(pic,tx+x+1,ty+y)<<4);
}
static uint8_t shown(unsigned x,unsigned y) {
    unsigned px=(havoc_raster_x+x)&255,py=(havoc_raster_y+y)&255;
    uint8_t b=pano_test_ram[0x4000+(px/2)*256+py];
    return px&1?b>>4:b&15;
}
static void frame(unsigned pic) {
    unsigned x,y;
    for(y=0;y<240;++y)for(x=0;x<200;++x) {
        uint8_t want=pixel(pic,pano_camera_x+x,pano_camera_y+y),got=shown(x,y);
        if(want!=got) {
            fprintf(stderr,"pixel mismatch pic%u camera%u,%u display%u,%u at%u,%u: %u != %u origin%u,%u h%d tile%u/%u\n",
                pic,pano_camera_x,pano_camera_y,havoc_raster_x,havoc_raster_y,x,y,got,want,
                origin_x,origin_y,horizontal,strip_tile,strip_tiles);abort();
        }
    }
}
static void guarded_frame(unsigned pic) {
    int x,y; frame(pic);
    for(y=-4;y<244;++y)for(x=-4;x<204;++x)
        assert(shown((unsigned)x,(unsigned)y)==pixel(pic,pano_camera_x+x,pano_camera_y+y));
}
static void service(unsigned jobs) {
    uint8_t done=pano_step((uint8_t)jobs,ticks-50000u);
    assert(done<=jobs);
}
static void travel(unsigned count,int dx,int dy) {
    unsigned i;
    for(i=0;i<count;++i) {
        if(!(i&1))pano_request(dx,dy);
        pano_tick();service(3);frame(0);
    }
}
static void transition_test(unsigned parity) {
    unsigned i,x,y;uint16_t old_x,old_y;
    while((havoc_raster_x&1)!=parity) {pano_request(1,0);pano_tick();service(3);}
    old_x=pano_camera_x;old_y=pano_camera_y;
    assert(pano_prefetch(0x80000u,640,768));
    assert(!pano_prefetch(0x80000u,640,768));
    for(i=0;i<300 && !pano_can_transition();++i) {service(3);frame(0);}
    assert(pano_can_transition());
    pano_transition();
    for(i=0;i<400 && !pano_transition_done();++i) {
        pano_tick();service(3);
        if(pano_transition_done())break;
        for(y=0;y<240;++y)for(x=0;x<200;++x) {
            unsigned relative=slide+x;
            uint8_t want=relative<slide_length?pixel(0,old_x+relative,old_y+y):
                pixel(1,next.sx+relative-slide_length,next.sy+y);
            if(shown(x,y)!=want) {
                fprintf(stderr,"transition mismatch parity%u progress%u col%u part%u at%u,%u\n",
                    parity,slide,seed_column,seed_part,x,y);abort();
            }
        }
    }
    assert(pano_transition_done() && !pano_busy());frame(1);
}
int main(void) {
    uint8_t snapshot[32768];unsigned i;
    asset(0,0,640,768);asset(0x80000,1,640,768);
    assert(pano_init(0,640,768));frame(0);
    memcpy(snapshot,pano_test_ram+0x4000,sizeof(snapshot));
    assert(pano_step(3,ticks)==0);
    assert(!memcmp(snapshot,pano_test_ram+0x4000,sizeof(snapshot)));
    for(i=0;i<300;++i) {pano_request(1,1);pano_tick();frame(0);}
    assert(pano_camera_x==origin_x+56 && pano_camera_y==origin_y+16);
    travel(2400,1,1);assert(pano_camera_x==440 && pano_camera_y==528);
    travel(2800,-1,-1);assert(pano_camera_x==0 && pano_camera_y==0);
    travel(500,1,1);travel(300,-1,1);travel(300,1,-1);
    transition_test(1);
    assert(pano_init(0,640,768));transition_test(0);
    asset(0x100000u,2,905,1086);
    assert(pano_init(0x100000u,905,1086));
    for(i=0;i<4000;++i) {
        if(!(i&1))pano_request(1,1);
        pano_tick();service(3);frame(2);
    }
    assert(pano_camera_x==705 && pano_camera_y==846);
    for(i=0;i<4500;++i) {
        if(!(i&1))pano_request(-1,-1);
        pano_tick();service(3);frame(2);
    }
    assert(pano_camera_x==0 && pano_camera_y==0);
    assert(!pano_tilt_guard(1));
    assert(pano_init(0,640,768)); assert(pano_tilt_guard(1));
    assert(!pano_prefetch(0x80000,640,768));
    for(i=0;i<2200;++i) {
        if(!(i&1))pano_request(1,1);
        pano_tick();service(3);guarded_frame(0);
    }
    assert(pano_camera_x==436 && pano_camera_y==524);
    assert(pano_tilt_guard(0));travel(30,1,1);
    assert(pano_camera_x==440 && pano_camera_y==528);
    printf("Panorama runtime: diagonal/reversal, both cache wraps, readiness/deadline gates, odd/even slide and retirement pass (%u transfers).\n",transfers);
    return 0;
}
