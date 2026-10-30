/* Execute production main.c's selection/controller functions with modeled
 * loader, audio-boundary, and panorama ownership; no hardware is accessed. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#define AUDIO_SAMPLE_RATE 16000UL
#include "../target-c64/include/platform.h"
static uint8_t test_ram[65536];
#undef REG
#define REG(address) test_ram[(uint16_t)(address)]
#define main firmware_main
#include "../target-c64/main.c"
#undef main

volatile uint8_t disk_error,disk_busy,havoc_fault,audio_running,audio_audible_track;
volatile uint16_t disk_chunks,havoc_underruns,havoc_minimum_lead,havoc_dma_ticks;
volatile uint32_t produced,consumed;
uint32_t audio_audible_start,audio_audible_frames;
uint8_t audio_event,audio_next_valid;
AudioTrack audio_decode_track;
volatile uint16_t pano_camera_x,pano_camera_y,pano_max_job_ticks;
volatile uint8_t pano_prefetched,pano_retired;
volatile int8_t havoc_tilt_k;
static DiskBundle bundles[2];
static uint8_t available[2],pin_count[2],loading_slot,loading_id;
static uint8_t picture_busy,can_transition,boundary_waiting,descriptor_valid=1;
static uint8_t queue_id,queue_cut;
static unsigned request_calls,reads,prefetches,queues,releases,transitions;
static uint32_t remaining_frames=160000,now=100000,prefetch_address;
uint32_t timer_read(void) { return now; }
void sampler_stop(void) {}
const DiskBundle *disk_ready(uint8_t track) {
    unsigned i;for(i=0;i<2;++i)if(available[i]&&bundles[i].track==track)return &bundles[i];
    return 0;
}
uint8_t disk_request(uint8_t track) {
    ++request_calls;if(disk_ready(track))return 1;
    if(disk_busy)return 0;
    loading_slot=pin_count[0]?1:0;if(pin_count[loading_slot])return 0;
    loading_id=track;available[loading_slot]=0;disk_busy=1;return 1;
}
void disk_pin(uint8_t slot) { assert(slot<2);++pin_count[slot]; }
void disk_release(uint8_t slot) { assert(slot<2&&pin_count[slot]);--pin_count[slot];++releases; }
uint8_t audio_track_read(AudioTrack *track,uint32_t address,uint32_t bytes,uint8_t id) {
    ++reads;assert(address==0x1000u+(uint32_t)id*65536&&bytes==45064);
    if(!descriptor_valid)return 0;
    memset(track,0,sizeof *track);track->id=id;return 1;
}
uint32_t audio_remaining(void) { return remaining_frames; }
void audio_queue(const AudioTrack *track,uint8_t cut) {
    if(audio_next_valid||boundary_waiting)return;
    ++queues;queue_id=track->id;queue_cut=cut;audio_next_valid=1;
}
uint8_t pano_busy(void) { return picture_busy; }
uint8_t pano_prefetch(uint32_t address,uint16_t w,uint16_t h) {
    assert(w==905&&h==1086);if(picture_busy)return 0;
    picture_busy=1;can_transition=pano_retired=0;prefetch_address=address;++prefetches;return 1;
}
uint8_t pano_can_transition(void) { return can_transition; }
void pano_transition(void) { assert(can_transition&&picture_busy);++transitions; }
uint8_t pano_transition_done(void) { return pano_retired; }
void pano_request(int8_t x,int8_t y) { (void)x;(void)y; }
uint8_t pano_tilt_guard(uint8_t enabled) { (void)enabled;return 1; }
void havoc_tilt_tick(uint8_t flatten) { if(flatten)havoc_tilt_k=0; }
static void set_bundle(unsigned slot,unsigned track) {
    bundles[slot].slot=(uint8_t)slot;bundles[slot].track=(uint8_t)track;
    bundles[slot].audio_reu=0x1000u+track*65536u;bundles[slot].audio_bytes=45064;
    bundles[slot].panorama_reu=0x800000u+track*65536u;
    bundles[slot].width=905;bundles[slot].height=1086;available[slot]=1;
}
static void complete_load(void) { set_bundle(loading_slot,loading_id);disk_busy=0; }
static void frame(void) {
    selection_step(now-1000);
    if(prepared&&pano_can_transition()&&!audio_next_valid)audio_queue(&pending_track,manual_cut);
}
int main(void) {
    uint8_t first=musicdisk_playlist[0],next=musicdisk_playlist[1],previous=musicdisk_playlist[20];
    unsigned saved;
    audio_audible_track=first;active_slot=0;requested=next;old_slot=pending_slot=255;
    set_bundle(0,first);disk_pin(0);automatic=0;
    frame();assert(disk_busy&&loading_id==next);saved=request_calls;
    frame();assert(request_calls==saved);complete_load();
    selection_step(now-10);assert(!descriptor_ready&&!reads);
    frame();assert(descriptor_ready&&!prepared&&pending_track.id==next&&reads==1);
    /* Replace a decoded descriptor before art prefetch. The replacement must
     * fetch its own header; it cannot accidentally queue the previous track. */
    saved=request_calls;test_ram[0xdc01]=(uint8_t)~16u;controls();
    assert(requested==previous&&manual_cut&&!descriptor_ready&&!disk_busy&&request_calls==saved);
    /* Controls only record the request. Loading needs more than400ticks of
     * foreground headroom, including at the exact admission boundary. */
    selection_step(now-400);assert(!disk_busy&&request_calls==saved);
    selection_step(now-401);assert(disk_busy&&request_calls==saved+1);
    frame();assert(!prepared&&reads==1);complete_load();havoc_tilt_k=10;frame();
    assert(descriptor_ready&&!prepared&&reads==2&&!pin_count[1]);
    havoc_tilt_k=0;frame();
    assert(prepared&&pending_track.id==previous&&reads==2&&pin_count[1]==1);
    assert(prefetch_address==bundles[1].panorama_reu&&prefetches==1&&!queues);
    can_transition=1;frame();assert(queues==1&&queue_id==previous&&queue_cut);
    audio_next_valid=0;boundary_waiting=1;frame();frame();
    assert(queues==1&&prepared&&active_slot==0&&pin_count[0]&&pin_count[1]);
    boundary_waiting=0;audio_audible_track=previous;audio_event=1;frame();
    assert(!prepared&&active_slot==1&&old_slot==0&&transitions==1&&!releases);
    saved=request_calls;frame();assert(request_calls==saved&&pin_count[0]);
    pano_retired=1;picture_busy=0;can_transition=0;frame();
    assert(old_slot==255&&releases==1&&!pin_count[0]&&pin_count[1]);
    assert(requested==first&&pending_track.id==first&&descriptor_ready);
    /* A failed descriptor cannot publish/pin a new incoming bundle. */
    descriptor_ready=0;descriptor_valid=0;remaining_frames=1;frame();
    assert(disk_error==DISK_HEADER&&!prepared&&pin_count[0]==0);
    descriptor_valid=1;frame();assert(prepared&&pin_count[0]==1&&pending_track.id==first);
    puts("PASS: production selection descriptor replacement, deadline guard, manual cut, boundary requeue, old/new slot pins, wipe retirement and bad-header recovery.");
    return 0;
}
