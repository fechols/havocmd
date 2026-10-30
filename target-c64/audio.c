#include "include/audio.h"
#include "include/platform.h"
#include "include/audio_step.h"
#include "include/audio_fast.h"
#include "include/raster.h"

#ifdef HAVOC_AUDIO_NATIVE_TEST
extern uint8_t audio_test_ram[65536];
#define AUDIO_RAM(address) (audio_test_ram+(uint16_t)(address))
#else
#define AUDIO_RAM(address) ((uint8_t *)(uint16_t)(address))
#endif
#define INPUT AUDIO_RAM(0xe600)
#define BOOK AUDIO_RAM(0xe680)
volatile uint8_t havoc_audio_chunk_state[6],havoc_audio_chunk_units[6],havoc_audio_chunk_reset[6];
uint8_t havoc_audio_codes_hi[6],havoc_audio_pcm_hi[6],havoc_audio_lut_hi[6];
static const uint8_t chunk_page[6]={0xc0,0xc3,0xc6,0xc9,0xe0,0xe3};
static uint8_t chunk_lut[6],current_lut;
AudioTrack audio_decode_track;
static AudioTrack queued;
volatile uint8_t havoc_fault,audio_running,audio_audible_track;
volatile uint16_t havoc_underruns,havoc_minimum_lead=32767,havoc_dma_ticks;
volatile uint32_t produced,consumed;
uint32_t audio_audible_start,audio_audible_frames;
uint8_t audio_event,audio_next_valid;
static uint8_t fill_index,drain_index,reset_next,cut_next,boundary_pending,boundary_id;
static uint16_t segment,segment_left;
static uint32_t source_frame,stream_position,planned,last_tick,clock_remainder;
static uint32_t boundary,boundary_frames;
static uint16_t drain_cost=280,fill_cost=170,book_fill_cost=250;

static uint32_t le32(const uint8_t *p) {
    return (uint32_t)p[0]|((uint32_t)p[1]<<8)|((uint32_t)p[2]<<16)|((uint32_t)p[3]<<24);
}
/* Quotients are bounded:8 bits for segment groups,11 bits for one clock
 * interval. The remainder stays exact without a resident general32-bit divide.
 * Callers prove the bound or reject a leftover>=divisor. */
static __attribute__((section(".tailcode"),noinline)) uint16_t divide_bounded(
    uint32_t *remainder,uint32_t trial,uint8_t bits) {
    uint16_t quotient=0;
    do {
        quotient<<=1;
        if(*remainder>=trial) { *remainder-=trial;++quotient; }
        trial>>=1;
    } while(--bits);
    return quotient;
}
uint8_t audio_track_read(AudioTrack *t,uint32_t address,uint32_t bytes,uint8_t id) {
    uint32_t groups,stream_bytes;uint16_t short_groups;
    reu_fetch(address,INPUT,32);
    if(INPUT[0]!='D'||INPUT[1]!='L'||INPUT[2]!='T'||INPUT[3]!='5'||
       le32(INPUT+4)!=AUDIO_SAMPLE_RATE||INPUT[16]!=8||INPUT[17]||INPUT[20]!=3||INPUT[21]!=2)return 0;
    t->frames=le32(INPUT+8);t->books=(uint16_t)INPUT[18]|((uint16_t)INPUT[19]<<8);
    if(!t->frames||(t->frames&3)||!t->books)return 0;
    t->stream=32+(uint32_t)t->books*32;
    stream_bytes=(t->frames/4)*3;
    if(le32(INPUT+12)!=stream_bytes ||
       t->stream+stream_bytes+(stream_bytes+1023UL)/1024UL!=bytes)return 0;
    groups=t->frames/4;
    short_groups=divide_bounded(&groups,(uint32_t)t->books<<7,8);
    if(!short_groups||groups>=t->books)return 0;
    t->long_segments=(uint16_t)groups;t->short_segment=short_groups<<2;
    t->address=address;t->id=id;return 1;
}
static __attribute__((section(".lowcode"),noinline)) void source_reset(void) {
    source_frame=stream_position=0;segment=0;reset_next=1;current_lut=255;
    segment_left=audio_decode_track.short_segment+(audio_decode_track.long_segments?4:0);
}
__attribute__((section(".disk_init"),noinline)) void audio_init(const AudioTrack *track) {
    uint8_t i;audio_decode_track=*track;source_reset();
    for(i=0;i<6;++i) {
        havoc_audio_chunk_state[i]=0;havoc_audio_codes_hi[i]=chunk_page[i];
        havoc_audio_pcm_hi[i]=chunk_page[i]+1;havoc_audio_lut_hi[i]=chunk_page[i]+2;
    }
    for(i=0;i<4;++i)havoc_audio_predictors[i]=0;
    havoc_audio_position=0;havoc_audio_quota=0;havoc_audio_current=5;
    audio_audible_track=track->id;audio_audible_frames=track->frames;
    audio_audible_start=produced=consumed=planned=0;
    fill_index=drain_index=audio_next_valid=cut_next=boundary_pending=audio_event=0;
}
void audio_queue(const AudioTrack *track,uint8_t cut) {
    if(audio_next_valid||boundary_pending)return;
    queued=*track;cut_next=cut;audio_next_valid=1;
}
static __attribute__((noinline)) void prepare_chunk(uint8_t slot) {
    uint8_t i,n,frames,table=current_lut,used=0;
    uint16_t left;
    if(source_frame==audio_decode_track.frames||(audio_next_valid&&cut_next)) {
        /* One pending boundary suffices: every packaged song exceeds ring duration. */
        if(boundary_pending) {havoc_fault=7;return;}
        if(audio_next_valid) {audio_decode_track=queued;audio_next_valid=cut_next=0;}
        boundary=planned;boundary_frames=audio_decode_track.frames;
        boundary_id=audio_decode_track.id;boundary_pending=1;source_reset();table=255;
    }
    if(!segment_left) {
        ++segment;table=255;
        segment_left=audio_decode_track.short_segment+(segment<audio_decode_track.long_segments?4:0);
    }
    left=segment_left;frames=left>128?128:(uint8_t)left;
    /* The validated book partition sums exactly to frames, with nonzero
     * multiples of four per segment, so this chunk already ends in bounds. */
    n=(frames>>2)*3;
    /* The producer visits each book in order. Retain its LUT even when all
     * decoded chunks have drained; only advancing the source can replace it.
     * Live chunks still pin their own LUT until their PCM has been drained. */
    if(table==255) {
        for(i=0;i<6;++i)if(havoc_audio_chunk_state[i]!=HAVOC_AUDIO_FREE)
            used|=(uint8_t)(1u<<chunk_lut[i]);
        table=0;while(used&1){used>>=1;++table;}
        reu_fetch(audio_decode_track.address+32+(uint32_t)segment*32,BOOK,32);
        audio_lut_fast(chunk_page[table]+2);
    }
    current_lut=table;chunk_lut[slot]=table;
    havoc_audio_lut_hi[slot]=chunk_page[table]+2;
    reu_fetch(audio_decode_track.address+audio_decode_track.stream+stream_position,INPUT,n);
    audio_unpack_fast(((uint16_t)n<<8)|havoc_audio_codes_hi[slot]);
    source_frame+=frames;planned+=frames;stream_position+=n;segment_left-=frames;
    havoc_audio_chunk_units[slot]=frames>>1;havoc_audio_chunk_reset[slot]=reset_next;reset_next=0;
    havoc_audio_chunk_state[slot]=HAVOC_AUDIO_READY;
}
void audio_service(uint32_t deadline) {
    uint8_t count=0;uint16_t begin=audio_timer16(),start,elapsed;
    while(count++<6&&havoc_audio_chunk_state[drain_index]==HAVOC_AUDIO_DONE) {
        uint16_t bytes=(uint16_t)havoc_audio_chunk_units[drain_index]*4;
        uint16_t at=(uint16_t)(produced*2),first=bytes;
        uint8_t *pcm=AUDIO_RAM((uint16_t)havoc_audio_pcm_hi[drain_index]<<8);
        if(produced-consumed+bytes/2>=32768UL)break;
        start=audio_timer16();
        if(deadline&&(int16_t)(start-(uint16_t)deadline)<=(int16_t)(drain_cost+32))break;
        if(at&&(uint32_t)at+bytes>65536UL)first=(uint16_t)(65536UL-at);
        reu_stash(pcm,RING_ADDRESS+at,first);
        if(first<bytes)reu_stash(pcm+first,RING_ADDRESS,bytes-first);
        produced+=bytes/2;havoc_audio_chunk_state[drain_index]=HAVOC_AUDIO_FREE;
        if(++drain_index==6)drain_index=0;
        elapsed=start-audio_timer16();if(elapsed>drain_cost)drain_cost=elapsed;
    }
    count=0;
    while(count++<6&&havoc_audio_chunk_state[fill_index]==HAVOC_AUDIO_FREE) {
        uint8_t new_book=current_lut==255||!segment_left||
            source_frame==audio_decode_track.frames||(audio_next_valid&&cut_next);
        uint16_t cost=new_book?book_fill_cost:fill_cost;
        start=audio_timer16();
        if(deadline&&(int16_t)(start-(uint16_t)deadline)<=(int16_t)(cost+32))break;
        prepare_chunk(fill_index);if(havoc_fault)return;
        if(++fill_index==6)fill_index=0;
        elapsed=start-audio_timer16();
        if(new_book) {if(elapsed>book_fill_cost)book_fill_cost=elapsed;}
        else if(elapsed>fill_cost)fill_cost=elapsed;
    }
    havoc_dma_ticks=begin-audio_timer16();
}
void audio_clock(void) {
    uint32_t now=timer_read(),elapsed=last_tick-now,scaled;
    last_tick=now;if(!audio_running)return;
    if(elapsed>65535UL){havoc_fault=6;sampler_stop();audio_running=0;return;}
    scaled=elapsed*32768UL+clock_remainder;
    /* Startup validates a measured ring interval in[1800000,2400000] ticks.
     * Thus trial fits32 bits and elapsed<=65535 implies a quotient<2048. */
    consumed+=divide_bounded(&scaled,sampler_ticks_per_ring<<10,11);clock_remainder=scaled;
    if(produced-consumed<=256UL||produced-consumed>=32768UL) {
        ++havoc_underruns;havoc_fault=5;sampler_stop();audio_running=0;return;
    }
    if(produced-consumed<havoc_minimum_lead)havoc_minimum_lead=(uint16_t)(produced-consumed);
    if(boundary_pending&&(int32_t)(consumed-boundary)>=0) {
        audio_audible_track=boundary_id;audio_audible_start=boundary;audio_audible_frames=boundary_frames;
        boundary_pending=0;audio_event=1;
    }
    if(consumed>=0x01000000UL) {
        consumed-=0x01000000UL;produced-=0x01000000UL;planned-=0x01000000UL;
        audio_audible_start-=0x01000000UL;if(boundary_pending)boundary-=0x01000000UL;
    }
}
__attribute__((section(".tailcode"),noinline)) uint32_t audio_remaining(void) {
    uint32_t elapsed=consumed-audio_audible_start;
    return elapsed<audio_audible_frames?audio_audible_frames-elapsed:0;
}
void audio_start(uint16_t divider) {
    (void)divider;
    consumed=clock_remainder=0;sampler_arm();last_tick=timer_read();audio_running=1;
}
