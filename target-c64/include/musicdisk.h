#ifndef HAVOC_MUSICDISK_H
#define HAVOC_MUSICDISK_H
#include <stdint.h>
#define DISK_SLOT_BYTES 0x500000UL
#define DISK_SLOT0 0x001000UL
#define DISK_SLOT1 0x501000UL
#define DISK_NO_TRACK 255u
enum { DISK_OK=0,DISK_NO_UCI=1,DISK_NOT_FOUND=2,DISK_HEADER=3,
       DISK_RANGE=4,DISK_IO=5,DISK_SHORT_READ=6,DISK_TIMEOUT=7,DISK_NO_SLOT=8 };
typedef struct {
    uint32_t audio_reu,audio_bytes,frames,panorama_reu,panorama_bytes;
    uint16_t width,height,storage_width,storage_height;
    uint8_t track,slot;
} DiskBundle;
extern volatile uint8_t disk_error,disk_busy;
extern volatile uint16_t disk_chunks;
/* Startup uses $4000-$57FF as scratch. Four headers are validated and the
 * verified directory is stored at REU0. No manual preload or firmware writes. */
uint8_t disk_init(void);
uint8_t disk_request(uint8_t track);
void disk_step(void); /* bounded; caller schedules with audio headroom */
const DiskBundle *disk_ready(uint8_t track);
void disk_pin(uint8_t slot); /* reference count: playback and wipe may own it */
void disk_release(uint8_t slot);
/* A fault preserves pinned bundles. Timed-out native I/O stays busy until
 * completion is acknowledged; its destination must not be recycled. */
#endif
