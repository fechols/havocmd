/* Continuous 16 kHz stereo playback with one active and one prefetched bundle. */
#include <stdint.h>
#include "include/platform.h"
#include "include/raster.h"
#include "include/audio.h"
#include "include/musicdisk.h"
#include "include/musicdisk_assets.h"
#include "include/panorama.h"
#if defined(HAVOC_PRODUCTION) && MUSICDISK_ASSETS_READY && MUSICDISK_SAMPLE_RATE != AUDIO_SAMPLE_RATE
#error Repack the full content at the player sample rate before compiling.
#endif

volatile uint16_t havoc_late_frames,havoc_late_lines_total;
static AudioTrack pending_track;
static uint8_t active_slot,old_slot=255,pending_slot=255,requested,descriptor_ready;
static uint8_t prepared,manual_cut,automatic=1,muted,old_keys=255,frame_divider;
static int8_t dx=1,dy=1;
static __attribute__((section(".tailcode"),noinline)) uint8_t adjacent(uint8_t id,int8_t delta) {
    uint8_t i;
    for(i=0;i<MUSICDISK_TRACK_COUNT;++i)if(musicdisk_playlist[i]==id)break;
    if(i==MUSICDISK_TRACK_COUNT)i=0;
    if(delta>0){if(++i==MUSICDISK_TRACK_COUNT)i=0;}
    else i=i?i-1:MUSICDISK_TRACK_COUNT-1;
    return musicdisk_playlist[i];
}
static uint8_t keyrow(uint8_t mask) {
    REG(0xdc02)=255;REG(0xdc03)=0;REG(0xdc00)=mask;return REG(0xdc01);
}
static uint8_t camera_near_edge(void) {
    /* Every authored panorama is905x1086. Level the view48pixels before an
     * edge: even manual one-pixel/frame motion leaves enough time for the
     * maximum40-frame flatten. Stay level through the edge and departure. */
    return (uint16_t)(pano_camera_x-48u)>609u ||
           (uint16_t)(pano_camera_y-48u)>750u;
}
static void camera_step(void) {
    if(!pano_camera_x)dx=1;else if(pano_camera_x>=705)dx=-1;
    if(!pano_camera_y)dy=1;else if(pano_camera_y>=846)dy=-1;
    pano_request(dx,dy);
}
static void controls(void) {
    uint8_t keys=keyrow(254),pressed=old_keys&~keys,j;
    old_keys=keys;
    if(!prepared&&!audio_next_valid&&!pano_busy()&&(pressed&48)) {
        requested=adjacent(audio_audible_track,(pressed&32)?1:-1);
        manual_cut=1;descriptor_ready=0;
    }
    if(pressed&64)automatic=1;
    if(pressed&8){muted=!muted;REG(0xdf21)=muted?0:32;REG(0xdf41)=muted?0:32;}
    REG(0xdc02)=0;REG(0xdc00)=255;j=REG(0xdc00);
    if((j&15)!=15) {
        automatic=0;
        if(!(j&1))pano_request(0,-1);if(!(j&2))pano_request(0,1);
        if(!(j&4))pano_request(-1,0);if(!(j&8))pano_request(1,0);
    }
    if(!(j&16))automatic=1;
    if(automatic&&!(++frame_divider&1)&&!pano_busy()) {
        camera_step();
    }
}
static void selection_step(uint32_t deadline) {
    const DiskBundle *bundle;
    if(audio_event) {
        audio_event=0;
        if(prepared&&audio_audible_track==requested) {
            old_slot=active_slot;active_slot=pending_slot;pending_slot=255;
            pano_transition();prepared=descriptor_ready=manual_cut=0;
            requested=adjacent(audio_audible_track,1);
        }
    }
    if(old_slot!=255&&pano_transition_done()) {
        disk_release(old_slot);old_slot=255;
    }
    if(old_slot!=255||prepared||pano_busy())return;
    bundle=disk_ready(requested);
    if(!bundle) {
        if(!disk_busy&&(int32_t)(timer_read()-deadline)>400)disk_request(requested);
        return;
    }
    if(!descriptor_ready) {
        if((int32_t)(timer_read()-deadline)<350)return;
        if(!audio_track_read(&pending_track,bundle->audio_reu,bundle->audio_bytes,bundle->track)) {
            disk_error=DISK_HEADER;return;
        }
        descriptor_ready=1;
    }
    if(!havoc_tilt_k&&(manual_cut||audio_remaining()<4UL*AUDIO_SAMPLE_RATE)) {
        if(pano_prefetch(bundle->panorama_reu,bundle->width,bundle->height)) {
            disk_pin(bundle->slot);pending_slot=bundle->slot;prepared=1;
        }
    }
}
static __attribute__((section(".tailcode"),noinline)) void fail(uint8_t color) {
    sampler_stop();REG(0xd020)=color;for(;;){}
}
static __attribute__((section(".disk_init"),noinline)) void boot_relocate(void) {
    uint16_t n;
    extern uint8_t __high_start[],__high_end[],__high_load[];
    extern uint8_t __mid_start[],__mid_end[],__mid_load[];
    extern uint8_t __tail_start[],__tail_end[],__tail_load[];
    for(n=0;n<(uint16_t)(__high_end-__high_start);++n)__high_start[n]=__high_load[n];
    for(n=0;n<(uint16_t)(__mid_end-__mid_start);++n)__mid_start[n]=__mid_load[n];
    for(n=0;n<(uint16_t)(__tail_end-__tail_start);++n)__tail_start[n]=__tail_load[n];
}
static __attribute__((section(".disk_init"),noinline)) const DiskBundle *boot_prepare(void) {
    uint16_t n,divider;
    const DiskBundle *first;
    if(!disk_init())fail(2);
    requested=musicdisk_playlist[0];
    if(!disk_request(requested))fail(2);
    while(!(first=disk_ready(requested))) {
        disk_step();if(disk_error&&!disk_busy)fail(2);
    }
    if(!audio_track_read(&pending_track,first->audio_reu,first->audio_bytes,first->track))fail(8);
    for(n=0;n<256;++n)REG(0xe600+n)=0;
    for(n=0;n<256;++n)reu_stash((void*)0xe600,RING_ADDRESS+((uint32_t)n<<8),256);
    if(REG(0xdf21)!=0x10)fail(4);
    divider=sampler_calibrate();
    if(!divider||sampler_ticks_per_ring<1800000UL||sampler_ticks_per_ring>2400000UL)fail(8);
    audio_init(&pending_track);active_slot=first->slot;disk_pin(active_slot);
    while(produced<32640UL) {
        audio_service(0);if(havoc_fault)fail(8);
        for(n=0;n<384;++n)havoc_raster_work();
    }
    sampler_prepare(divider);
    return first;
}
int main(void) {
    uint16_t lead_line;uint8_t late;
    const DiskBundle *first;
    platform_init();REG(0xd020)=0;
    boot_relocate();
    first=boot_prepare();
    /* Production raster initialization builds only its lookup table. The
     * one-time overlay must remain intact until pano_init returns from it. */
    havoc_raster_init();
    if(!pano_init(first->panorama_reu,first->width,first->height))fail(8);
    requested=adjacent(audio_audible_track,1);descriptor_ready=0;disk_request(requested);
    lead_line=havoc_raster_top>=8?havoc_raster_top-8:263u+havoc_raster_top-8;
    while(raster_line()==lead_line){}
    while(raster_line()!=lead_line){}
    audio_start(0);late=havoc_raster_late_lines;
    for(;;) {
        uint32_t before=timer_read(),deadline;
        havoc_raster_frame();deadline=timer_read()-1430UL;
        havoc_late_lines_total+=(uint8_t)(havoc_raster_late_lines-late);
        late=havoc_raster_late_lines;
        audio_clock();
        {
            int8_t previous_tilt=havoc_tilt_k;
            uint8_t flatten=pano_busy()||manual_cut||camera_near_edge()||
                audio_remaining()<6UL*AUDIO_SAMPLE_RATE;
            if(!flatten&&!pano_tilt_guard(1))flatten=1;
            havoc_tilt_tick(flatten);
            if(previous_tilt!=havoc_tilt_k)deadline+=110u;
            if(flatten&&!havoc_tilt_k)pano_tilt_guard(0);
        }
        if(!havoc_fault)audio_service(deadline);
        if(!havoc_fault&&(int32_t)(timer_read()-deadline)>180) {
            selection_step(deadline);
            if(prepared&&pano_can_transition()&&!audio_next_valid)
                audio_queue(&pending_track,manual_cut);
        }
        if((int32_t)(timer_read()-deadline)>180)controls();
        if((int32_t)(timer_read()-deadline)>180)disk_step();
        pano_step(3,deadline);pano_tick();
        if(before-timer_read()>26000UL)++havoc_late_frames;
    }
}
