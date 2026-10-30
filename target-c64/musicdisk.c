/* Native Ultimate DOS loader. Only read-only DOS commands are issued here.
 * A LOAD_REU owns its destination until the final reply has been acknowledged. */
#include "include/platform.h"
#include "include/uci.h"
#include "include/musicdisk.h"
#include "include/musicdisk_assets.h"
#ifdef HAVOC_DISK_TEST
#define HIGH
#define EARLY
#define BOOT
#define MID
extern uint8_t disk_test_ram[65536];
extern uint32_t disk_test_directory_crc;
#define HEADER (disk_test_ram+0x4000)
#define MOUNTS (disk_test_ram+0x5000)
#define EXPECTED_CRC disk_test_directory_crc
#else
#define HIGH __attribute__((noinline))
#define EARLY __attribute__((section(".earlycode"),noinline))
#define BOOT __attribute__((section(".disk_init"),noinline))
#define MID __attribute__((section(".midcode"),noinline))
#define HEADER ((uint8_t *)0x4000)
#define MOUNTS ((uint8_t *)0x5000)
#define EXPECTED_CRC MUSICDISK_DIRECTORY_CRC
#endif
#define VOLUME_BYTES 0x1000000UL
#define HEADER_BYTES 4096u
#define CHUNK_BYTES 16384u
volatile uint8_t disk_error,disk_busy;
volatile uint16_t disk_chunks;
static char folder[96];
static uint8_t command[112],entry[64];
static DiskBundle slots[2];
static uint8_t pins[2],ready[2],selected,asset,volume,state,file_open;
static uint32_t source,remaining,destination;
static uint16_t requested;
enum { OPEN=1,SEEK,LOAD,CLOSE,DONE,FAILED };

static EARLY uint32_t get32(const uint8_t *p) {
    return (uint32_t)p[0]|((uint32_t)p[1]<<8)|((uint32_t)p[2]<<16)|((uint32_t)p[3]<<24);
}
static EARLY void put32(uint8_t *p,uint32_t value) {
    uint8_t n;for(n=0;n<4;++n) { p[n]=(uint8_t)value;value>>=8; }
}
static EARLY void copy_bytes(void *out,const uint8_t *in,uint8_t n) {
    uint8_t *p=(uint8_t *)out;while(n--) *p++=*in++;
}
static HIGH void fetch_entry(uint8_t track,uint8_t panorama) {
    reu_fetch(64UL+(uint16_t)(track+(panorama?MUSICDISK_TRACK_COUNT:0))*64,entry,64);
}
static EARLY void source_entry(void) {
    volume=entry[3];source=get32(entry+4);remaining=get32(entry+8);
}
static EARLY uint16_t open_command(uint8_t index) {
    uint8_t i=0,n=3;const char *name="havocmd-1.reu";
    command[0]=1;command[1]=2;command[2]=1; /* FA_READ */
    while(folder[i])command[n++]=(uint8_t)folder[i++];
    i=0;while(name[i]) { command[n++]=(uint8_t)(i==8?'1'+index:name[i]);++i; }
    return n;
}
static HIGH uint8_t start(uint8_t opcode,uint16_t length) {
    command[0]=1;command[1]=opcode;
    return uci_start(command,length,entry,sizeof entry,0);
}
static MID void next_command(void) {
    if(state==OPEN) { uci_start(command,open_command(volume),entry,sizeof entry,0);return; }
    if(state==SEEK) { put32(command+2,source);start(6,6);return; }
    if(state==LOAD) {
        /* End on a64KiB boundary as well as the16MiB volume boundary. This
         * bounds every native request without a32-bit subtraction here. */
        uint16_t edge=(uint16_t)(0-(uint16_t)source);
        requested=CHUNK_BYTES;
        if(remaining<requested)requested=(uint16_t)remaining;
        if(edge&&edge<requested)requested=edge;
        put32(command+2,destination);put32(command+6,requested);start(0x21,10);return;
    }
    start(3,2);
}
HIGH const DiskBundle *disk_ready(uint8_t track) {
    uint8_t i;for(i=0;i<2;++i)if(ready[i]&&slots[i].track==track)return &slots[i];
    return 0;
}
HIGH void disk_pin(uint8_t slot) { if(slot<2&&pins[slot]!=255)++pins[slot]; }
HIGH void disk_release(uint8_t slot) { if(slot<2&&pins[slot])--pins[slot]; }
HIGH uint8_t disk_request(uint8_t track) {
    DiskBundle *b;
    if(track>=MUSICDISK_TRACK_COUNT)return 0;
    if(disk_ready(track))return 1;
    if(disk_busy||uci_busy)return 0;
    selected=0;if(pins[0])selected=1;
    if(pins[selected])return 0;
    b=&slots[selected];ready[selected]=0;b->track=track;b->slot=selected;
    b->audio_reu=selected?DISK_SLOT1:DISK_SLOT0;
    fetch_entry(track,0);copy_bytes(&b->audio_bytes,entry+8,4);copy_bytes(&b->frames,entry+20,4);
    fetch_entry(track,1);copy_bytes(&b->panorama_bytes,entry+8,4);
    copy_bytes(&b->width,entry+16,2);copy_bytes(&b->height,entry+20,2);
    copy_bytes(&b->storage_width,entry+24,2);copy_bytes(&b->storage_height,entry+28,2);
    /* The CRC-checked startup directory already proved both lengths and their
     * sum fit the slot. This directory is retained outside all I/O targets. */
    b->panorama_reu=b->audio_reu+b->audio_bytes;
    fetch_entry(track,0);source_entry();destination=b->audio_reu;
    disk_error=0;disk_busy=1;file_open=0;asset=0;state=OPEN;next_command();return 1;
}
HIGH void disk_step(void) {
    if(!disk_busy)return;
    uci_step();
    if(uci_error)disk_error=uci_error==UCI_TIMEOUT?DISK_TIMEOUT:DISK_IO;
    /* A timeout is not cancellation: retain the slot and drain the reply. */
    if(!uci_complete)return;
    uci_complete=0;
    if(!uci_status_ok()&&!disk_error)disk_error=DISK_IO;
    if(state==OPEN&&uci_status_ok())file_open=1;
    if(state==CLOSE)file_open=0;
    if(disk_error) {
        if(file_open) { state=CLOSE;next_command();return; }
        state=FAILED;disk_busy=0;return;
    }
    if(state==OPEN)state=SEEK;
    else if(state==SEEK)state=LOAD;
    else if(state==LOAD) {
        if(!uci_transfer_matches(requested,destination)) {
            disk_error=DISK_SHORT_READ;state=CLOSE;next_command();return;
        }
        source+=requested;remaining-=requested;destination+=requested;++disk_chunks;
        if(!remaining||source==VOLUME_BYTES)state=CLOSE;
    } else if(state==CLOSE) {
        if(remaining) { ++volume;source=HEADER_BYTES;state=OPEN; }
        else if(!asset) {
            asset=1;fetch_entry(slots[selected].track,1);source_entry();state=OPEN;
        } else { ready[selected]=1;state=DONE;disk_busy=0;return; }
    }
    next_command();
}

/* Everything below runs before $6000-$BFFF becomes the panorama cache. */
static BOOT uint8_t transact(uint16_t length,uint8_t *out,uint16_t capacity) {
    if(!uci_start(command,length,out,capacity,0))return 0;
    while(uci_busy) { uci_step();if(uci_error)return 0; }
    return !uci_error;
}
static BOOT uint8_t simple(uint8_t opcode) {
    command[0]=1;command[1]=opcode;
    return transact(2,entry,sizeof entry)&&uci_status_ok();
}
static BOOT uint32_t crc32(const uint8_t *data,uint16_t count,uint8_t header) {
    uint32_t crc=0xffffffffUL;uint16_t i;uint8_t bit,value;
    for(i=0;i<count;++i) {
        value=header&&i>=32&&i<36?0:data[i];crc^=value;
        for(bit=0;bit<8;++bit)crc=(crc>>1)^((crc&1)?0xedb88320UL:0);
    }
    return crc^0xffffffffUL;
}
static BOOT uint8_t check_header(uint8_t index,uint8_t count) {
    uint8_t i;uint16_t j;uint8_t *p;
    const char *magic="HVMDVOL";
    for(i=0;i<7;++i)if(HEADER[i]!=(uint8_t)magic[i])return 0;
    if(HEADER[7]||HEADER[8]!=MUSICDISK_FORMAT_VERSION||HEADER[9]||
       HEADER[10]||HEADER[11]!=16||HEADER[12]!=index||HEADER[13]!=count||!count||count>4||
       HEADER[14]!=MUSICDISK_ASSET_COUNT||HEADER[15]||get32(HEADER+16)!=VOLUME_BYTES||
       get32(HEADER+20)<HEADER_BYTES||get32(HEADER+20)>VOLUME_BYTES||
       get32(HEADER+24)!=EXPECTED_CRC||crc32(HEADER+64,4032,0)!=EXPECTED_CRC||
       crc32(HEADER,HEADER_BYTES,1)!=get32(HEADER+32))return 0;
    for(i=36;i<64;++i)if(HEADER[i])return 0;
    for(j=64+MUSICDISK_ASSET_COUNT*64;j<HEADER_BYTES;++j)if(HEADER[j])return 0;
    for(i=0;i<MUSICDISK_ASSET_COUNT;++i) {
        uint32_t offset,length,capacity,w,h,sw,sh;
        p=HEADER+64+(uint16_t)i*64;offset=get32(p+4);length=get32(p+8);
        if(p[0]!=i%MUSICDISK_TRACK_COUNT||p[1]||p[2]!=(i<MUSICDISK_TRACK_COUNT?1:2)||
           p[3]>=count||offset<HEADER_BYTES||offset>=VOLUME_BYTES||!length)return 0;
        capacity=VOLUME_BYTES-offset+(uint32_t)(count-1-p[3])*(VOLUME_BYTES-HEADER_BYTES);
        if(length>capacity)return 0;
        if(i<MUSICDISK_TRACK_COUNT) {
            if(get32(p+16)!=MUSICDISK_SAMPLE_RATE||!get32(p+20)||
               get32(p+24)!=2||get32(p+28)!=3||length>DISK_SLOT_BYTES)return 0;
            if(get32(p+8)+get32(p+MUSICDISK_TRACK_COUNT*64+8)>DISK_SLOT_BYTES)return 0;
        } else {
            w=get32(p+16);h=get32(p+20);sw=get32(p+24);sh=get32(p+28);
            if(w<256||h<256||w>65535||h>65535||sw<w||sh<h||sw>65535||sh>65535||
               (sw&15)||(sh&7)||length!=32768UL+(sw*sh)/2)return 0;
        }
    }
    return 1;
}
static BOOT uint8_t set_folder(const uint8_t *path,uint16_t length) {
    uint16_t i;
    while(length&&path[length-1]==0)--length;
    if(!length||length>94)return 0;
    for(i=0;i<length;++i)folder[i]=(char)path[i];
    if(folder[length-1]!='/')folder[length++]='/';
    folder[length]=0;return 1;
}
static BOOT uint8_t try_folder(void) {
    uint8_t v,valid,count=1;
    for(v=0;v<count;++v) {
        if(!transact(open_command(v),entry,sizeof entry)||!uci_status_ok())return 0;
        valid=simple(7)&&uci_response_length>=4&&get32(entry)==VOLUME_BYTES;
        if(valid) {
            command[0]=1;command[1]=4;command[2]=0;command[3]=16;
            valid=transact(4,HEADER,HEADER_BYTES)&&uci_response_length==HEADER_BYTES&&
                  (!uci_status_length||uci_status_ok())&&check_header(v,v?count:HEADER[13]);
        }
        if(!simple(3))return 0;
        if(!valid) { disk_error=DISK_HEADER;return 0; }
        if(v==0) { count=HEADER[13];reu_stash(HEADER,0,HEADER_BYTES); }
    }
    disk_error=0;return 1;
}
static BOOT uint8_t path_folder(uint8_t opcode) {
    command[0]=1;command[1]=opcode;
    if(!transact(2,HEADER,256)||!uci_status_ok())return 0;
    return set_folder(HEADER,uci_response_length)&&try_folder();
}
BOOT uint8_t disk_init(void) {
    uint8_t count=0,i,j;uint16_t n;
    disk_error=DISK_NOT_FOUND;disk_busy=0;disk_chunks=0;
    for(i=0;i<2;++i) { pins[i]=ready[i]=0;slots[i].track=DISK_NO_TRACK; }
    if(path_folder(0x12))return 1; /* Current DOS path, if already useful. */
    if(uci_error)goto fault;
    if(path_folder(0x17))return 1; /* Configured Home; read-only path change. */
    if(uci_error)goto fault;
    command[0]=1;command[1]=0x11;command[2]='/';
    if(!transact(3,entry,sizeof entry)||!uci_status_ok()||!simple(0x13))goto fault;
    command[0]=1;command[1]=0x14;
    if(!uci_start(command,2,entry,sizeof entry,1))goto fault;
    while(uci_busy) {
        uci_step();if(uci_error)goto fault;
        if(uci_packet_ready) {
            n=uci_response_length;
            if(n>1&&n<=64&&(entry[0]&16)&&count<32&&entry[1]!='.') {
                uint8_t *name=MOUNTS+(uint16_t)count*64;
                for(j=1;j<n;++j)name[j-1]=entry[j];
                name[n-1]=0;++count;
            }
            uci_accept_packet();
        }
    }
    if(!uci_status_ok())goto fault;
    for(i=0;i<count;++i) {
        const uint8_t *name=MOUNTS+(uint16_t)i*64;
        const char *suffix="/havocmd/";
        folder[0]='/';j=1;while(*name)folder[j++]=(char)*name++;
        n=0;while(suffix[n])folder[j++]=suffix[n++];folder[j]=0;
        if(try_folder())return 1;
        if(uci_error)goto fault;
    }
    return 0;
fault:
    if(uci_error==UCI_ABSENT)disk_error=DISK_NO_UCI;
    else if(uci_error==UCI_TIMEOUT)disk_error=DISK_TIMEOUT;
    else disk_error=DISK_IO;
    return 0;
}
