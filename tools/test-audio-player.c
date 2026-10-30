/* Production audio.c with modeled RAM/REU and an independent scalar callback.
 * test-audio-player.py also checks the current assembly callback against the
 * existing linked NMOS6502 oracle. Neither test contacts hardware. */
#define HAVOC_AUDIO_NATIVE_TEST 1
#define AUDIO_SAMPLE_RATE 16000UL
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../target-c64/include/audio_step.h"
uint8_t audio_test_ram[65536];
#undef havoc_audio_predictors
#undef havoc_audio_position
#undef havoc_audio_quota
#undef havoc_audio_current
#define havoc_audio_predictors (audio_test_ram+0xf0)
#define havoc_audio_position audio_test_ram[0xf4]
#define havoc_audio_quota audio_test_ram[0xf5]
#define havoc_audio_current audio_test_ram[0xf6]
#include "../target-c64/audio.c"

#define TRACKS 3
#define MAX_FRAMES 70004u
static uint8_t reu[0x1000000],expected[TRACKS][MAX_FRAMES*2];
static AudioTrack tracks[TRACKS];
static uint32_t asset_bytes[TRACKS],ticks=0xffffffffUL,logical_consumed;
static unsigned transfers,stashes,armed,stopped,reset_calls,division_cases;
volatile uint32_t sampler_ticks_per_ring=2097152; /* exact64 modeled ticks/frame */
static const uint32_t addresses[TRACKS]={0x1000,0x100000,0x200000};
static const uint32_t frame_counts[TRACKS]={60000,64004,70004};
static const uint16_t book_counts[TRACKS]={180,192,209};
uint32_t timer_read(void) { return ticks; }
void sampler_stop(void) { ++stopped; }
void sampler_arm(void) { ++armed; }
void reu_fetch(uint32_t source,void *out,uint16_t n) {
    unsigned i;assert(n&&source+n<=sizeof reu);
    for(i=0;i<TRACKS;++i)
        if(source>=addresses[i]&&source+n<=addresses[i]+asset_bytes[i])break;
    assert(i<TRACKS); /* no read past the complete compressed source */
    assert((uint8_t *)out>=audio_test_ram&&(uint8_t *)out+n<=audio_test_ram+65536);
    memcpy(out,reu+source,n);++transfers;
}
void reu_stash(const void *in,uint32_t destination,uint16_t n) {
    assert(n&&destination>=RING_ADDRESS&&destination+n<=RING_ADDRESS+65536);
    assert((const uint8_t *)in>=audio_test_ram&&(const uint8_t *)in+n<=audio_test_ram+65536);
    memcpy(reu+destination,in,n);++stashes;
}
static int32_t clamp16(int32_t value) {
    return value<-32768?-32768:value>32767?32767:value;
}
static uint8_t pcm8(int32_t value) {
    int32_t rounded=value+128;
    rounded=rounded>=0?rounded/256:-((-rounded+255)/256);
    return (uint8_t)(rounded>127?127:rounded);
}
void havoc_raster_work(void) {
    uint8_t slot=havoc_audio_current,sub,channel;
    if(--havoc_audio_quota>=128) {
        havoc_audio_quota=0;
        if(havoc_audio_chunk_state[slot]==HAVOC_AUDIO_PLAYING)
            havoc_audio_chunk_state[slot]=HAVOC_AUDIO_DONE;
        slot=(uint8_t)((slot+1)%6);
        if(havoc_audio_chunk_state[slot]!=HAVOC_AUDIO_READY)return;
        havoc_audio_current=slot;havoc_audio_chunk_state[slot]=HAVOC_AUDIO_PLAYING;
        havoc_audio_quota=havoc_audio_chunk_units[slot];havoc_audio_position=0;
        if(havoc_audio_chunk_reset[slot]) { memset(havoc_audio_predictors,0,4);++reset_calls; }
        return;
    }
    for(sub=0;sub<2;++sub) {
        uint8_t position=havoc_audio_position;
        uint8_t code=audio_test_ram[((uint16_t)havoc_audio_codes_hi[slot]<<8)+position];
        for(channel=0;channel<2;++channel) {
            uint8_t *predictor=havoc_audio_predictors+channel*2;
            uint16_t lut=((uint16_t)havoc_audio_lut_hi[slot]<<8)+channel*128+code;
            int16_t delta=(int16_t)((uint16_t)audio_test_ram[lut]|((uint16_t)audio_test_ram[lut+64]<<8));
            int16_t previous=(int16_t)((uint16_t)predictor[0]|((uint16_t)predictor[1]<<8));
            int32_t next=clamp16((int32_t)previous+delta);
            predictor[0]=(uint8_t)next;predictor[1]=(uint8_t)((uint16_t)next>>8);
            audio_test_ram[((uint16_t)havoc_audio_pcm_hi[slot]<<8)+position+channel]=pcm8(next);
        }
        havoc_audio_position=(uint8_t)(position+2);
    }
}
static void put32(uint8_t *p,uint32_t value) {
    unsigned i;for(i=0;i<4;++i) { p[i]=(uint8_t)value;value>>=8; }
}
static uint8_t source_code(unsigned track,uint32_t frame) {
    return (uint8_t)((frame*13+frame/31+track*7)&63);
}
static void make_assets(void) {
    static const int16_t deltas[8]={-32768,-128,-1,0,1,127,128,32767};
    unsigned t,b,ch,k;uint32_t f;
    for(t=0;t<TRACKS;++t) {
        uint32_t frames=frame_counts[t],stream=32+book_counts[t]*32u;
        uint8_t *p=reu+addresses[t];int32_t predictor[2]={0,0};
        uint32_t short_frames=(frames/4/book_counts[t])*4,extra=(frames/4)%book_counts[t];
        uint32_t segment_end=short_frames+(extra?4:0);unsigned segment=0;
        asset_bytes[t]=stream+(frames/4)*3+((frames/4)*3+1023)/1024;
        memcpy(p,"DLT5",4);put32(p+4,AUDIO_SAMPLE_RATE);put32(p+8,frames);put32(p+12,(frames/4)*3);p[16]=8;
        p[18]=(uint8_t)book_counts[t];p[19]=(uint8_t)(book_counts[t]>>8);p[20]=3;p[21]=2;
        for(b=0;b<book_counts[t];++b)for(ch=0;ch<2;++ch)for(k=0;k<8;++k) {
            int16_t delta=deltas[(k+t*3+ch*5+b*7)%8];
            uint8_t *at=p+32+b*32+ch*16+k*2;
            at[0]=(uint8_t)delta;at[1]=(uint8_t)((uint16_t)delta>>8);
        }
        for(f=0;f<frames;f+=4) {
            uint32_t packed=0;
            for(k=0;k<4;++k)packed|=(uint32_t)source_code(t,f+k)<<(k*6);
            p[stream+(f/4)*3]=(uint8_t)packed;
            p[stream+(f/4)*3+1]=(uint8_t)(packed>>8);
            p[stream+(f/4)*3+2]=(uint8_t)(packed>>16);
        }
        {
            uint8_t checksum=0;uint32_t size=(frames/4)*3;
            for(f=0;f<size;++f) {
                checksum^=p[stream+f];
                for(k=0;k<8;++k)checksum=(uint8_t)((checksum<<1)^((checksum&128)?7:0));
                if((f&1023)==1023||f+1==size)p[stream+size+f/1024]=checksum;
            }
        }
        for(f=0;f<frames;++f) {
            if(f==segment_end) { ++segment;segment_end+=short_frames+(segment<extra?4:0); }
            for(ch=0;ch<2;++ch) {
                unsigned code=(source_code(t,f)>>(ch*3))&7;
                int16_t delta=deltas[(code+t*3+ch*5+segment*7)%8];
                predictor[ch]=clamp16(predictor[ch]+delta);
                expected[t][f*2+ch]=pcm8(predictor[ch]);
            }
        }
        assert(audio_track_read(&tracks[t],addresses[t],asset_bytes[t],(uint8_t)t));
        assert(tracks[t].frames==frames&&tracks[t].books==book_counts[t]);
    }
}
static void pump(void) {
    unsigned i;audio_service(0);assert(!havoc_fault);
    for(i=0;i<400;++i)havoc_raster_work();
    audio_service(0);assert(!havoc_fault);
}
static void begin(unsigned track) {
    unsigned guard=1000;
    memset(audio_test_ram,0,sizeof audio_test_ram);havoc_fault=audio_running=0;
    havoc_underruns=0;reset_calls=0;logical_consumed=0;audio_init(&tracks[track]);
    while(produced<32640&&guard) { pump();--guard; }
    assert(produced>=32640);
    assert(produced-consumed<32768);audio_start(390);assert(audio_running);
}
static void consume(unsigned track,uint32_t offset,uint16_t frames) {
    unsigned f,ch;
    assert(offset+frames<=frame_counts[track]);assert(produced-consumed>frames+256u);
    for(f=0;f<frames;++f)for(ch=0;ch<2;++ch) {
        uint8_t actual=reu[RING_ADDRESS+(uint16_t)((consumed+f)*2+ch)];
        uint8_t wanted=expected[track][(offset+f)*2+ch];
        if(actual!=wanted) {
            fprintf(stderr,"PCM mismatch at timeline%u track%u source%u channel%u: %u != %u\n",
                    logical_consumed+f,track,offset+f,ch,actual,wanted);abort();
        }
    }
    ticks-=(uint32_t)frames*64;audio_clock();logical_consumed+=frames;assert(!havoc_fault);
}
static void natural_sequence(void) {
    uint32_t offset;unsigned before;
    begin(0);before=stashes;pump();pump();assert(stashes==before); /* full ring is retained */
    audio_queue(&tracks[1],0);
    for(offset=0;offset<frame_counts[0]-1;) {
        uint16_t n=(uint16_t)(frame_counts[0]-1-offset);if(n>128)n=128;
        pump();consume(0,offset,n);offset+=n;assert(!audio_event&&audio_audible_track==0);
        if(boundary_pending) { audio_queue(&tracks[0],1);assert(!audio_next_valid&&boundary_id==1); }
    }
    assert(boundary_pending&&boundary==frame_counts[0]);
    consume(0,offset,1);assert(audio_event&&audio_audible_track==1&&audio_remaining()==frame_counts[1]);
    audio_event=0;
    for(offset=0;offset<frame_counts[1]-1;) {
        uint16_t n=(uint16_t)(frame_counts[1]-1-offset);if(n>128)n=128;
        pump();consume(1,offset,n);offset+=n;assert(!audio_event);
    }
    consume(1,offset,1);assert(audio_event&&audio_audible_track==1);
    assert(audio_audible_start==frame_counts[0]+frame_counts[1]);audio_event=0;
    for(offset=0;offset<32768;offset+=128) { pump();consume(1,offset,128); }
    assert(reset_calls>=3&&havoc_underruns==0);
}
static void manual_cut(void) {
    uint32_t cut,offset;unsigned i,guard=1000;
    memset(audio_test_ram,0,sizeof audio_test_ram);havoc_fault=audio_running=0;
    havoc_underruns=0;reset_calls=0;logical_consumed=0;audio_init(&tracks[0]);
    audio_service(0);cut=planned;
    /* Only the first chunk finishes: old book0 chunks still own their LUT
     * when incoming track1/book0 is prepared in the newly freed slot. */
    for(i=0;i<66;++i)havoc_raster_work();
    audio_queue(&tracks[1],1);audio_service(0);
    assert(boundary_pending&&boundary==cut&&cut<frame_counts[0]);
    while(produced<32640&&guard) { pump();--guard; }
    assert(produced>=32640);audio_start(390);
    for(offset=0;offset<cut;) {
        uint16_t n=(uint16_t)(cut-offset);if(n>128)n=128;
        pump();consume(0,offset,n);offset+=n;
    }
    assert(audio_event&&audio_audible_track==1);audio_event=0;
    for(offset=0;offset<32768;offset+=128) { pump();consume(1,offset,128); }
    assert(!havoc_underruns);
}
static void segmented_and_rebase(void) {
    uint32_t offset,delta;uint32_t old_remaining;
    begin(2);
    for(offset=0;offset<32760;) {
        uint16_t n=(uint16_t)(32760-offset);if(n>128)n=128;
        pump();consume(2,offset,n);offset+=n;
    }
    delta=0x00ff8000UL;
    produced+=delta;consumed+=delta;planned+=delta;audio_audible_start+=delta;
    if(boundary_pending)boundary+=delta;
    old_remaining=audio_remaining();assert(consumed==0x00fffff8UL);
    pump();consume(2,offset,8);offset+=8;
    assert(consumed==0&&audio_remaining()==old_remaining-8);
    while(offset<frame_counts[2]) {
        uint16_t n=(uint16_t)(frame_counts[2]-offset);if(n>128)n=128;
        pump();consume(2,offset,n);offset+=n;
    }
    assert(audio_event&&audio_audible_track==2&&audio_remaining()==frame_counts[2]);
}
static void pending_boundary_rebase(void) {
    uint32_t offset,delta=0x00ff8000UL;
    begin(0);audio_queue(&tracks[1],0);
    for(offset=0;offset<32760;) {
        uint16_t n=(uint16_t)(32760-offset);if(n>128)n=128;
        pump();consume(0,offset,n);offset+=n;
    }
    assert(boundary_pending&&boundary_id==1&&boundary==frame_counts[0]);
    produced+=delta;consumed+=delta;planned+=delta;audio_audible_start+=delta;boundary+=delta;
    consume(0,offset,8);offset+=8;
    assert(consumed==0&&boundary_pending&&boundary==frame_counts[0]-32768);
    while(offset<frame_counts[0]) {
        uint16_t n=(uint16_t)(frame_counts[0]-offset);if(n>128)n=128;
        pump();consume(0,offset,n);offset+=n;
    }
    assert(audio_event&&audio_audible_track==1&&audio_remaining()==frame_counts[1]);
}
static void arithmetic_oracle(void) {
    static const uint32_t divisors[]={1800000,2094545,2097152,2400000};
    static const uint16_t intervals[]={0,1,32,65,4096,65535};
    static const uint16_t books[]={1,2,7,128,65535};
    unsigned i,j,k;uint32_t seed=0x640024;
    for(i=0;i<4;++i)for(j=0;j<6;++j)for(k=0;k<4;++k) {
        uint32_t d=divisors[i],old=k==0?0:k==1?1:k==2?d/2:d-1;
        uint32_t value=(uint32_t)intervals[j]*32768+old,remainder=value;
        assert(divide_bounded(&remainder,d<<10,11)==value/d);
        assert(remainder==value%d);++division_cases;
    }
    for(i=0;i<4096;++i) {
        uint32_t d=divisors[i&3],value,remainder;
        seed=seed*1664525u+1013904223u;
        value=(seed&65535)*32768u+(seed%d);remainder=value;
        assert(divide_bounded(&remainder,d<<10,11)==value/d);
        assert(remainder==value%d);++division_cases;
    }
    for(i=0;i<5;++i)for(j=0;j<7;++j) {
        uint32_t d=books[i],n=j==0?0:j==1?d-1:j==2?d:j==3?d*255:j==4?d*256-1:j==5?d*256:0xffffffffUL;
        uint32_t remainder=n,q=n/d;if(q>255)q=255;
        assert(divide_bounded(&remainder,d<<7,8)==q);
        assert(remainder==n-q*d);++division_cases;
    }
    /* A ratio that cannot fit the8-bit segment quotient is rejected rather
     * than silently truncating its group count. The byte envelope stays valid. */
    {
        AudioTrack invalid;uint8_t *header=reu+addresses[0];
        uint16_t saved=(uint16_t)header[18]|((uint16_t)header[19]<<8);
        uint32_t stream_bytes=frame_counts[0]/4*3;
        header[18]=1;header[19]=0;
        assert(!audio_track_read(&invalid,addresses[0],64+stream_bytes+(stream_bytes+1023)/1024,0));
        header[18]=(uint8_t)saved;header[19]=(uint8_t)(saved>>8);
    }
}
static void failure_and_deadline_guards(void) {
    AudioTrack invalid;uint8_t saved;unsigned before;
    saved=reu[addresses[0]+21];reu[addresses[0]+21]=1;
    assert(!audio_track_read(&invalid,addresses[0],asset_bytes[0],0));reu[addresses[0]+21]=saved;
    saved=reu[addresses[0]+16];reu[addresses[0]+16]=7;
    assert(!audio_track_read(&invalid,addresses[0],asset_bytes[0],0));reu[addresses[0]+16]=saved;
    assert(!audio_track_read(&invalid,addresses[0],asset_bytes[0]-1,0));
    begin(0);before=transfers+stashes;audio_service(ticks-10);
    assert(transfers+stashes==before); /* insufficient deadline cannot begin DMA */
    ticks-=65536;audio_clock();assert(havoc_fault==6&&!audio_running&&stopped);
    begin(0);produced=consumed+256;ticks-=32;audio_clock();
    assert(havoc_fault==5&&havoc_underruns==1&&!audio_running);
}
int main(void) {
    make_assets();arithmetic_oracle();natural_sequence();manual_cut();segmented_and_rebase();pending_boundary_rebase();failure_and_deadline_guards();
    printf("PASS: production producer exact PCM, distinct-track book0 reuse/reset, full sources, ring wraps, boundary events, retry loop, queued-boundary guard, counter rebase, deadlines and underrun stops (%u REU operations; %u exact bounded-division cases).\n",transfers+stashes,division_cases);
    return 0;
}
