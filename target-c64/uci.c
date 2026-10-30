#include "include/platform.h"
#include "include/uci.h"
#ifdef HAVOC_DISK_TEST
extern uint8_t uci_test_read(uint16_t);
extern void uci_test_write(uint16_t,uint8_t);
#define READ(a) uci_test_read(a)
#define WRITE(a,v) uci_test_write(a,v)
#define HIGH
#define MID
#else
#define READ(a) REG(a)
#define WRITE(a,v) (REG(a)=(v))
#define HIGH __attribute__((section(".highcode")))
#define MID __attribute__((section(".midcode"),noinline))
#endif
volatile uint8_t uci_error,uci_busy,uci_complete,uci_packet_ready;
uint8_t uci_status[32],uci_status_length,uci_packet_last;
uint16_t uci_response_length;
static const uint8_t *command;
static uint8_t *response;
static uint8_t command_length,command_at;
static uint16_t response_capacity;
static uint8_t phase,streaming;
static uint32_t started;

__attribute__((noinline)) uint8_t uci_start(const uint8_t *bytes,uint16_t length,
                       uint8_t *out,uint16_t capacity,uint8_t stream) {
    if(uci_busy || length<2 || length>112) return 0;
    command=bytes;command_length=(uint8_t)length;command_at=0;
    response=out;response_capacity=capacity;streaming=stream;
    uci_error=uci_complete=uci_packet_ready=uci_packet_last=0;
    uci_status_length=0;uci_response_length=0;phase=0;
    started=timer_read();uci_busy=1;return 1;
}
HIGH void uci_accept_packet(void) {
    if(!uci_packet_ready) return;
    WRITE(0xdf1c,2); /* DATA_ACC only, never DMA/TRIGGER/IRQ. */
    uci_packet_ready=0;phase=2;
}
HIGH void uci_step(void) {
    uint8_t status,budget;
    if(!uci_busy || uci_packet_ready) return;
    if(READ(0xdf1d)!=0xc9) { uci_error=UCI_ABSENT;return; }
    if(started-timer_read()>30000000UL) uci_error=UCI_TIMEOUT;
    status=READ(0xdf1c);
    if(status&8) { uci_error=UCI_PROTOCOL;WRITE(0xdf1c,8); }
    if(phase==0) {
        if(status&0x3f) return;
        budget=16;
        while(budget-- && command_at<command_length)
            WRITE(0xdf1d,command[command_at++]);
        if(command_at==command_length) { WRITE(0xdf1c,1);phase=1; }
        return;
    }
    if(phase==2) {
        if(status&3) return;
        if(uci_packet_last) {
            if(status&0x30) return;
            uci_busy=0;uci_complete=1;return;
        }
        if((status&0x30)<0x20) return;
        if(streaming) uci_response_length=0;
        uci_status_length=0;phase=1;
    }
    if((status&1) || (status&0x30)<0x20) return;
    budget=32;
    while(budget-- && ((status=READ(0xdf1c))&0xc0)) {
        uint8_t value;
        if(status&0x80) {
            value=READ(0xdf1e);
            if(uci_response_length<response_capacity)
                response[uci_response_length++]=value;
            else uci_error=UCI_OVERFLOW;
        } else {
            value=READ(0xdf1f);
            if(uci_status_length<sizeof uci_status)
                uci_status[uci_status_length++]=value;
            else uci_error=UCI_OVERFLOW;
        }
    }
    status=READ(0xdf1c);
    if(status&0xc0) return;
    uci_packet_last=(status&0x30)==0x20;
    uci_packet_ready=1;
    if(!streaming) uci_accept_packet();
}
__attribute__((section(".lowcode"),noinline)) uint8_t uci_status_ok(void) {
    return uci_status_length>=3 && uci_status[0]=='0' &&
           uci_status[1]=='0' && uci_status[2]==',';
}
/* Native LOAD_REU replies use "$%6x BYTES LOADED TO REU $%6x".
 * Compare its two 24-bit fields by nibble instead of doing 32-bit arithmetic
 * for every character. Spaces are accepted only before the first digit. */
static MID uint8_t hex_matches(const uint8_t *p,const uint8_t *expected) {
    uint8_t i,c,n,wanted,seen=0;
    for(i=0;i<6;++i) {
        c=p[i];
        if(c==' ') { if(seen)return 0;n=0; }
        else {
            seen=1;
            if(c>='0'&&c<='9')n=c-'0';
            else if(c>='a'&&c<='f')n=c-'a'+10;
            else if(c>='A'&&c<='F')n=c-'A'+10;
            else return 0;
        }
        wanted=expected[(5-i)>>1];
        if(!(i&1))wanted>>=4;
        if(n!=(wanted&15))return 0;
    }
    return seen&&!expected[3];
}
MID uint8_t uci_transfer_matches(uint32_t count,uint32_t destination) {
    uint8_t i;
    const char *middle=" BYTES LOADED TO REU $";
    if(uci_response_length!=35||response[0]!='$')return 0;
    for(i=0;i<21;++i)if(response[i+7]!=(uint8_t)middle[i])return 0;
    return hex_matches(response+1,(const uint8_t *)&count)&&
           hex_matches(response+29,(const uint8_t *)&destination);
}
