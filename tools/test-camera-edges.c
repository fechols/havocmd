/* Run the actual main.c camera policy against the production cache streamer.
 * gcc -O2 -ffunction-sections -fdata-sections tools/test-camera-edges.c
 *     -Wl,--gc-sections -o /tmp/test-camera-edges
 * The instruction-level tilt suite separately verifies this angle schedule. */
#define main panorama_regression_main
#include "test-panorama.c"
#undef main
/* main.c also contains fixed-address startup/input code. Map its register
 * macro to the same host RAM instead of casting 16-bit addresses to pointers. */
#undef REG
#define REG(a) pano_test_ram[(uint16_t)(a)]
/* Keep startup/runtime-only program sections discardable in this host test. */
#define __attribute__(x)
#define main musicdisk_program_main
#include "../target-c64/main.c"
#undef main
#undef __attribute__
volatile int8_t havoc_tilt_k;
static uint8_t animation_clock;
static int8_t animation_direction=1;
static void angle_frame(uint8_t flatten) {
    ++animation_clock;
    if(flatten) {
        if(!(animation_clock&3)) {
            if(havoc_tilt_k>0)--havoc_tilt_k;
            else if(havoc_tilt_k<0)++havoc_tilt_k;
        }
    } else if(!(animation_clock&31)) {
        if(havoc_tilt_k==10)animation_direction=-1;
        else if(havoc_tilt_k==-10)animation_direction=1;
        havoc_tilt_k+=animation_direction;
    }
}
int main(void) {
    unsigned i,edges=0,resumed=0;uint16_t before_x,before_y;
    asset(0x100000u,2,905,1086);
    assert(pano_init(0x100000u,905,1086));
    dx=dy=1;havoc_tilt_k=10;assert(pano_tilt_guard(1));
    for(i=0;i<6500;++i) {
        uint8_t flatten=camera_near_edge();
        if(!flatten&&!pano_tilt_guard(1))flatten=1;
        angle_frame(flatten);
        if(flatten&&!havoc_tilt_k)assert(pano_tilt_guard(0));
        before_x=pano_camera_x;before_y=pano_camera_y;
        if(!(i&1))camera_step();
        pano_tick();service(3);
        if(margin)guarded_frame(2);else frame(2);
        if(!pano_camera_x){edges|=1;assert(!havoc_tilt_k);}
        if(pano_camera_x==705){edges|=2;assert(!havoc_tilt_k);}
        if(!pano_camera_y){edges|=4;assert(!havoc_tilt_k);}
        if(pano_camera_y==846){edges|=8;assert(!havoc_tilt_k);}
        if(before_x==0&&pano_camera_x>0)assert(dx==1);
        if(before_x==705&&pano_camera_x<705)assert(dx==-1);
        if(before_y==0&&pano_camera_y>0)assert(dy==1);
        if(before_y==846&&pano_camera_y<846)assert(dy==-1);
        if(edges==15&&!camera_near_edge()&&havoc_tilt_k)resumed=1;
    }
    assert(edges==15 && resumed);
    /* The approach zone has room for maximum-speed manual movement too. */
    for(i=0;i<4;++i) {
        pano_camera_x=350;pano_camera_y=400;
        if(i==0)pano_camera_x=47;
        if(i==1)pano_camera_x=658;
        if(i==2)pano_camera_y=47;
        if(i==3)pano_camera_y=799;
        assert(camera_near_edge());
    }
    puts("Camera policy: all four true artwork edges reached level, reversed, and departed; tilt resumed safely.");
    return 0;
}
