#ifndef HAVOC_AUDIO_H
#define HAVOC_AUDIO_H
#include <stdint.h>
typedef struct {
    uint32_t address, frames, stream;
    uint16_t books, short_segment, long_segments;
    uint8_t id;
} AudioTrack;
extern AudioTrack audio_decode_track;
extern volatile uint8_t havoc_fault, audio_running, audio_audible_track;
extern volatile uint16_t havoc_underruns, havoc_minimum_lead, havoc_dma_ticks;
extern volatile uint32_t produced, consumed;
extern uint32_t audio_audible_start, audio_audible_frames;
extern uint8_t audio_event, audio_next_valid;
uint8_t audio_track_read(AudioTrack *track,uint32_t address,uint32_t bytes,uint8_t id);
void audio_init(const AudioTrack *track);
void audio_queue(const AudioTrack *track,uint8_t cut);
void audio_service(uint32_t deadline);
void audio_clock(void);
void audio_start(uint16_t divider);
uint32_t audio_remaining(void);
#endif
