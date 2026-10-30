#ifndef HAVOC_AUDIO_FAST_H
#define HAVOC_AUDIO_FAST_H
#include <stdint.h>
/* Packed argument: low byte=destination code page; high byte=packed input
 * bytes (3..96, multiple of3). Input starts at$E600; odd output bytes untouched.
 * LUT argument is its page; the32-byte16-bit stereo book starts at$E680. */
#ifdef HAVOC_AUDIO_NATIVE_TEST
extern uint8_t audio_test_ram[65536];
extern uint32_t timer_read(void);
static uint16_t audio_timer16(void) { return (uint16_t)timer_read(); }
static void audio_unpack_fast(uint16_t argument) {
    uint8_t n=argument>>8,i,out=0,*dst=audio_test_ram+((argument&255u)<<8);
    for(i=0;i<n;i+=3) {
        uint8_t a=audio_test_ram[0xe600+i],b=audio_test_ram[0xe601+i],c=audio_test_ram[0xe602+i];
        dst[out]=a&63;dst[(uint8_t)(out+2)]=((a>>6)|(b<<2))&63;
        dst[(uint8_t)(out+4)]=((b>>4)|(c<<4))&63;dst[(uint8_t)(out+6)]=c>>2;
        out+=8;
    }
}
static void audio_lut_fast(uint8_t page) {
    uint8_t i,*dst=audio_test_ram+((uint16_t)page<<8),*book=audio_test_ram+0xe680;
    for(i=0;i<64;++i) {
        uint8_t l=(i&7)*2,r=16+(i>>3)*2;
        dst[i]=book[l];dst[i+64]=book[l+1];dst[i+128]=book[r];dst[i+192]=book[r+1];
    }
}
#else
void audio_unpack_fast(uint16_t argument);
void audio_lut_fast(uint8_t destination_page);
/* Foreground intervals must stay below32768CIA ticks; robust across low-byte
 * rollover without assuming timer-read latch behavior. Returns A=low/X=high. */
uint16_t audio_timer16(void);
#endif
#endif
