/* Host-side protocol model; never contacts a C64U. Run with:
 * cc -std=c99 -O2 -Wall -Wextra tools/test-disk-loader.c -o /tmp/havoc-disk-test
 * /tmp/havoc-disk-test
 */
#define HAVOC_DISK_TEST 1
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define command uci_command_pointer
#include "../target-c64/uci.c"
#undef command
#include "../target-c64/musicdisk.c"
uint8_t disk_test_ram[65536];
uint32_t disk_test_directory_crc;
static uint8_t reu[0x1000000],headers[4][4096],wire[896],answer[4096];
static uint16_t wire_count,answer_length,answer_at,packet_end;
static uint8_t reply_state,status_at,status_length,reply_mode,dir_index;
static uint8_t opened=255,short_read,hold_load,held,corrupt_header;
static uint32_t file_position,ticks=0xffffffffUL;
static unsigned pushes,loads,max_writes,max_reads,io_writes,io_reads;
static const char success[]="00,OK";

uint32_t timer_read(void) { return ticks--; }
void reu_fetch(uint32_t source,void *out,uint16_t count) {
    assert(source+count<=sizeof reu);memcpy(out,reu+source,count);
}
void reu_stash(const void *in,uint32_t destination,uint16_t count) {
    assert(destination+count<=sizeof reu);memcpy(reu+destination,in,count);
}
static void reply(const void *data,uint16_t length,uint8_t mode,uint8_t ok) {
    if(length&&data!=answer)memcpy(answer,data,length);
    answer_length=length;answer_at=0;packet_end=length>512?512:length;
    reply_mode=mode;status_at=0;status_length=ok?5:0;
    reply_state=length>512?0x30:0x20;
    if(reply_state==0x30)status_length=0;
}
static void directory_reply(void) {
    static const uint8_t dirs[2][6]={{16,'S','D',0,0,0},{16,'U','S','B','1','1'}};
    reply(dirs[dir_index],dir_index?6:3,2,dir_index);
    if(!dir_index)reply_state=0x30;
}
static void process(void) {
    char path[112],expected[112];uint8_t i;
    uint32_t length,dest,count;
    assert(wire_count>=2&&wire[0]==1);++pushes;
    switch(wire[1]) {
    case 0x12:reply("/",1,0,1);break;
    case 0x17:reply("/SD",3,0,1);break;
    case 0x11:assert(wire_count==3&&wire[2]=='/');reply(0,0,0,1);break;
    case 0x13:reply(0,0,0,1);break;
    case 0x14:dir_index=0;directory_reply();break;
    case 2:
        assert(wire[2]==1);memcpy(path,wire+3,wire_count-3);path[wire_count-3]=0;
        opened=255;
        for(i=0;i<4;++i) {
            snprintf(expected,sizeof expected,"/USB11/havocmd/havocmd-%u.reu",i+1);
            if(!strcmp(path,expected))opened=i;
        }
        file_position=0;reply(0,0,0,opened!=255);break;
    case 3:opened=255;reply(0,0,0,1);break;
    case 7:
        assert(opened<4);put32(answer,0x1000000);reply(answer,4,0,1);break;
    case 4:
        assert(opened<4&&wire[2]==0&&wire[3]==16&&file_position==0);
        memcpy(answer,headers[opened],4096);
        if(corrupt_header&&opened==2)answer[80]^=1;
        reply(answer,4096,1,0);break;
    case 6:assert(opened<4);file_position=get32(wire+2);reply(0,0,0,1);break;
    case 0x21:
        assert(opened<4&&wire_count==10);++loads;
        dest=get32(wire+2);length=get32(wire+6);count=length;
        assert(length&&length<=16384&&file_position>=4096&&file_position+length<=0x1000000);
        assert(dest>=DISK_SLOT0&&dest+length<=DISK_SLOT1+DISK_SLOT_BYTES);
        if(pins[0])assert(dest>=DISK_SLOT1);
        if(pins[1])assert(dest+length<=DISK_SLOT1);
        if(short_read) { --count;short_read=0; }
        memset(reu+dest,opened+1,count);file_position+=count;
        snprintf((char *)answer,sizeof answer,"$%6x BYTES LOADED TO REU $%6x",(unsigned)count,(unsigned)dest);
        reply(answer,(uint16_t)strlen((char *)answer),0,1);
        if(hold_load) { held=1;reply_state=0x10; }
        break;
    default:assert(!"unexpected or non-read-only DOS command");
    }
    wire_count=0;
}
uint8_t uci_test_read(uint16_t address) {
    if(address==0xdf1d)return 0xc9;
    if(address==0xdf1c) {
        if(held)return 0x10;
        return reply_state|(answer_at<packet_end?0x80:0)|(status_at<status_length?0x40:0);
    }
    ++io_reads;
    if(address==0xdf1e) { assert(answer_at<packet_end);return answer[answer_at++]; }
    assert(address==0xdf1f&&status_at<status_length);return success[status_at++];
}
void uci_test_write(uint16_t address,uint8_t value) {
    if(address==0xdf1d) { ++io_writes;assert(wire_count<sizeof wire);wire[wire_count++]=value;return; }
    assert(address==0xdf1c);
    if(value==1) { process();return; }
    assert(value==2&&reply_state>=0x20);
    assert(answer_at==packet_end&&status_at==status_length);
    if(reply_mode==2&&dir_index==0) { ++dir_index;directory_reply();return; }
    if(packet_end<answer_length) {
        packet_end+=512;if(packet_end>answer_length)packet_end=answer_length;
        reply_state=packet_end==answer_length?0x20:0x30;
        status_at=status_length=0;return;
    }
    reply_state=0;status_at=status_length=0;
}
static void build_headers(void) {
    uint8_t i,v;uint8_t *p;
    memset(headers,0,sizeof headers);memcpy(headers[0],"HVMDVOL",7);
    headers[0][8]=2;headers[0][11]=16;headers[0][13]=4;headers[0][14]=42;
    put32(headers[0]+16,0x1000000);put32(headers[0]+20,0x1000000);
    for(i=0;i<42;++i) {
        p=headers[0]+64+(uint16_t)i*64;p[0]=i%21;p[2]=i<21?1:2;p[3]=1;
        put32(p+4,0x10000UL+(uint32_t)i*0x10000UL);put32(p+8,i<21?20000:65536);
        put32(p+16,i<21?MUSICDISK_SAMPLE_RATE:256);put32(p+20,i<21?40000:256);
        put32(p+24,i<21?2:256);put32(p+28,i<21?3:256);
    }
    p=headers[0]+64;p[3]=0;put32(p+4,0x1000000-8192); /* cross a volume header */
    disk_test_directory_crc=crc32(headers[0]+64,4032,0);
    put32(headers[0]+24,disk_test_directory_crc);
    for(v=0;v<4;++v) {
        if(v)memcpy(headers[v],headers[0],4096);
        headers[v][12]=v;put32(headers[v]+32,crc32(headers[v],4096,1));
    }
}
static void step_checked(void) {
    io_reads=io_writes=0;disk_step();
    if(io_reads>max_reads)max_reads=io_reads;
    if(io_writes>max_writes)max_writes=io_writes;
    assert(io_reads<=32&&io_writes<=16);
}
static void finish(void) {
    unsigned guard=100000;while(disk_busy&&guard--)step_checked();assert(guard&& !disk_busy);
}
static void invalid_metadata(void) {
    uint32_t saved_crc=disk_test_directory_crc;
    uint8_t test;
    for(test=0;test<5;++test) {
        memcpy(HEADER,headers[0],4096);
        switch(test) {
        case 0:put32(HEADER+64+8,DISK_SLOT_BYTES);break; /* bundle too large */
        case 1:put32(HEADER+64+24,1);break; /* mono is not this format */
        case 2:put32(HEADER+64+21*64+24,255);break; /* storage smaller than image */
        case 3:HEADER[64+3]=4;break; /* unavailable volume */
        default:HEADER[64+42*64]=1;break; /* nonzero reserved directory bytes */
        }
        disk_test_directory_crc=crc32(HEADER+64,4032,0);
        put32(HEADER+24,disk_test_directory_crc);put32(HEADER+32,crc32(HEADER,4096,1));
        assert(!check_header(0,4)); /* valid CRCs cannot bypass structure checks */
    }
    disk_test_directory_crc=saved_crc;
}
int main(void) {
    const DiskBundle *current;unsigned before;
    build_headers();invalid_metadata();
    response=answer;uci_response_length=35;
    memcpy(answer,"$  4000 BYTES LOADED TO REU $501000",35);
    assert(uci_transfer_matches(16384,DISK_SLOT1));
    assert(!uci_transfer_matches(16383,DISK_SLOT1));
    assert(!uci_transfer_matches(16384,DISK_SLOT0));
    answer[4]=' ';assert(!uci_transfer_matches(16384,DISK_SLOT1));
    assert(disk_init());assert(!strcmp(folder,"/USB11/havocmd/"));
    assert(!memcmp(reu+64,headers[0]+64,4032));
    assert(disk_request(0));finish();assert(disk_error==0);
    current=disk_ready(0);assert(current&&current->audio_reu==DISK_SLOT0);
    assert(reu[DISK_SLOT0]==1&&reu[DISK_SLOT0+8191]==1&&reu[DISK_SLOT0+8192]==2);
    assert(current->panorama_reu==DISK_SLOT0+20000);disk_pin(current->slot);
    before=loads;assert(disk_request(1));assert(!disk_request(2));finish();
    assert(loads>before&&disk_ready(1)&&disk_ready(0)==current);
    disk_pin(1);assert(!disk_request(2));disk_release(1);
    short_read=1;assert(disk_request(2));finish();assert(disk_error==DISK_SHORT_READ);
    assert(disk_ready(0)==current&&!disk_ready(2));
    hold_load=1;assert(disk_request(2));while(!held)step_checked();ticks-=31000000;
    step_checked();assert(disk_error==DISK_TIMEOUT&&disk_busy&&!disk_request(3));
    assert(disk_ready(0)==current);held=hold_load=0;reply_state=0x20;finish();
    assert(disk_error==DISK_TIMEOUT&&!disk_ready(2));
    assert(disk_request(2));finish();assert(disk_ready(2)&&disk_ready(0)==current);
    disk_release(0);corrupt_header=1;assert(!disk_init()&&disk_error==DISK_HEADER);
    printf("PASS: USB11 discovery, four header CRCs, volume crossing, pinned slots, short reads, timeout drain/retry; max %u command / %u reply bytes per step (%u commands).\n",max_writes,max_reads,pushes);
    return 0;
}
