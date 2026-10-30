/* Register setup derived from HAVOC's MIT-licensed working Ultimate Audio backend. */
#include "include/platform.h"
#ifdef HAVOC_PRODUCTION
#define BOOT_ONLY __attribute__((section(".disk_init"),noinline))
#define MID __attribute__((section(".midcode"),noinline))
#else
#define BOOT_ONLY
#define MID
#endif
volatile uint32_t sampler_ticks_per_ring;
volatile uint16_t havoc_dma8_budget_ticks,havoc_dma16_budget_ticks;
__attribute__((section(".lowcode"),noinline)) void probe_dma(void) {
    uint8_t i;uint32_t start=timer_read();
    /* 1536 and 2560 bytes approximate total 8/16-bit stereo audio traffic per
     * NTSC frame. This measures transfer budget, not 16-bit audio playback. */
    for(i=0;i<10;++i) {
        reu_stash((const void*)0xe600,0x600000UL+(uint32_t)i*256,256);
        if(i==5)havoc_dma8_budget_ticks=(uint16_t)(start-timer_read());
    }
    havoc_dma16_budget_ticks=(uint16_t)(start-timer_read());
}
static void word(uint16_t a, uint16_t v) { REG(a)=v; REG(a+1)=v>>8; }
static BOOT_ONLY void be24(uint16_t a, uint32_t v) { REG(a)=v>>16; REG(a+1)=v>>8; REG(a+2)=v; }
static MID void transfer(uint32_t reu, uint16_t ram, uint16_t count, uint8_t command) {
    word(0xdf02,ram); REG(0xdf04)=reu; REG(0xdf05)=reu>>8; REG(0xdf06)=reu>>16;
    word(0xdf07,count); REG(0xdf09)=0; REG(0xdf0a)=0; REG(0xdf01)=command;
}
MID void reu_fetch(uint32_t source, void *destination, uint16_t count) {
    if(count) transfer(source,(uint16_t)destination,count,0x91);
}
MID void reu_stash(const void *source, uint32_t destination, uint16_t count) {
    if(count) transfer(destination,(uint16_t)source,count,0x90);
}
uint16_t raster_line(void) {
    uint8_t h,l; do { h=REG(0xd011)&128; l=REG(0xd012); } while(h!=(REG(0xd011)&128));
    return (uint16_t)l | ((uint16_t)h<<1);
}
BOOT_ONLY void platform_init(void) {
    __asm__("sei\ncld");
    REG(1)=0x35; REG(0xdc0d)=0x7f; REG(0xdd0d)=0x7f;
    (void)REG(0xdc0d); (void)REG(0xdd0d); REG(0xd01a)=0; REG(0xd019)=0xff;
    REG(0xd011)=0; REG(0xd015)=0; REG(0xd031)=0x8f;
    word(0xdd04,65535); word(0xdd06,65535); REG(0xdd0e)=0x11; REG(0xdd0f)=0x51;
}
#ifndef HAVOC_PRODUCTION
/* Production uses asm/timer.S; retain the original prototype path. */
static MID uint16_t timer_word(uint16_t a) {
    uint8_t hi,lo; do { hi=REG(a+1); lo=REG(a); } while(hi!=REG(a+1));
    return ((uint16_t)hi<<8)|lo;
}
MID uint32_t timer_read(void) {
    uint16_t hi,lo; do { hi=timer_word(0xdd06); lo=timer_word(0xdd04); } while(hi!=timer_word(0xdd06));
    return ((uint32_t)hi<<16)|lo;
}
#endif
void sampler_stop(void) {
    /* Gate-off holds the last DAC value; mute both channels as well. */
    REG(0xdf21)=0; REG(0xdf41)=0;
    REG(0xdf20)=0; REG(0xdf40)=0; REG(0xdf3f)=255;
}
static BOOT_ONLY void configure(uint16_t divider, uint32_t length, uint8_t repeat) {
    uint16_t voice;
    sampler_stop();
    for(voice=0xdf20;voice<=0xdf40;voice+=32) {
        REG(voice+1)=0; REG(voice+2)=voice==0xdf20?0:15;
        REG(voice+4)=1; be24(voice+5,RING_ADDRESS+(voice==0xdf40));
        be24(voice+9,length); REG(voice+14)=divider>>8; REG(voice+15)=divider;
        be24(voice+17,0); be24(voice+21,repeat?length:0);
    }
}
static BOOT_ONLY uint32_t measure(uint16_t divider, uint16_t frames) {
    uint32_t begin,elapsed; configure(divider,(uint32_t)frames*2,0);
    begin=timer_read(); REG(0xdf20)=0x45; REG(0xdf40)=0x41;
    do { elapsed=begin-timer_read(); if(elapsed>1500000UL) { sampler_stop(); return 0; } } while(!REG(0xdf20));
    sampler_stop(); return elapsed;
}
BOOT_ONLY uint16_t sampler_calibrate(void) {
    /* Compile-time 64-bit arithmetic avoids overflowing the 12,288-frame
     * measurement interval while leaving only 32-bit operations at runtime. */
    const uint32_t target_ticks=(uint32_t)((12288ULL*1022727ULL+AUDIO_SAMPLE_RATE/2)/AUDIO_SAMPLE_RATE);
    const uint16_t initial_divider=6250000UL/AUDIO_SAMPLE_RATE;
    uint16_t divider=initial_divider, candidate; uint32_t a,b,ticks,best,diff;
    a=measure(divider,4096); b=measure(divider,16384);
    if(!a || b<=a) return 0;
    ticks=b-a; best=ticks>target_ticks?ticks-target_ticks:target_ticks-ticks;
    /* NTSC 1,022,727 Hz CIA time; measure the hardware, including divider offset. */
    candidate=(uint16_t)(((uint32_t)divider*target_ticks+ticks/2)/ticks);
    if(candidate<initial_divider-16 || candidate>initial_divider+16) return 0;
    if(candidate!=divider) {
        a=measure(candidate,4096); b=measure(candidate,16384);
        if(!a || b<=a) return 0;
        diff=(b-a)>target_ticks?(b-a)-target_ticks:target_ticks-(b-a);
        if(diff<best) { divider=candidate; ticks=b-a; }
    }
    /* 32768 / 12288 = 8/3; exact measured ring duration is retained. */
    sampler_ticks_per_ring=(ticks*8+1)/3;
    return divider;
}
BOOT_ONLY void sampler_prepare(uint16_t divider) {configure(divider,RING_BYTES,1);}
void sampler_arm(void) {
    REG(0xdf21)=32; REG(0xdf41)=32;
    REG(0xdf20)=0x43; REG(0xdf40)=0x43;
}
void sampler_start(uint16_t divider) {sampler_prepare(divider);sampler_arm();}
