#ifndef HAVOC_AUDIO_STEP_H
#define HAVOC_AUDIO_STEP_H

#include <stdint.h>

#define HAVOC_AUDIO_CHUNKS 6
#define HAVOC_AUDIO_FREE 0
#define HAVOC_AUDIO_READY 1
#define HAVOC_AUDIO_PLAYING 2
#define HAVOC_AUDIO_DONE 3

/* Reserve $F0-$F6 in the linker. C's imaginary registers must not use them. */
#define havoc_audio_predictors ((volatile uint8_t *)0x00f0)
#define havoc_audio_position (*(volatile uint8_t *)0x00f4)
#define havoc_audio_quota (*(volatile uint8_t *)0x00f5)
#define havoc_audio_current (*(volatile uint8_t *)0x00f6)

extern volatile uint8_t havoc_audio_chunk_state[HAVOC_AUDIO_CHUNKS];
extern volatile uint8_t havoc_audio_chunk_units[HAVOC_AUDIO_CHUNKS];
extern volatile uint8_t havoc_audio_chunk_reset[HAVOC_AUDIO_CHUNKS];
extern uint8_t havoc_audio_codes_hi[HAVOC_AUDIO_CHUNKS];
extern uint8_t havoc_audio_pcm_hi[HAVOC_AUDIO_CHUNKS];
extern uint8_t havoc_audio_lut_hi[HAVOC_AUDIO_CHUNKS];

/*
 * Each chunk has three aligned 256-byte pages: codes, PCM, lookup table.
 * A code byte at even index 2*f is left_code | right_code<<3. Odd bytes are
 * unused. LUT offsets 0/64/128/192 hold left-low/left-high/right-low/right-high;
 * each of their 64 entries corresponds to that combined code.
 *
 * Chunks contain 1..64 two-frame units, never crossing a codebook boundary.
 * Fill all data and metadata before publishing READY. RESET requests predictors
 * zeroed at a song/loop boundary. Start CURRENT=5, QUOTA=0, all states FREE.
 * Callback changes READY->PLAYING, then PLAYING->DONE; foreground drains
 * UNITS*4 output bytes from DONE chunks before publishing FREE.
 *
 * Exactly 256 nominal NMOS6502 cycles from entry through RTS, including idle
 * and page handoff. A work call emits two stereo frames; handoff/idle emits none.
 * A/X/Y and flags may change; caller must keep decimal mode clear. No I/O.
 * More calls in blanking are allowed. CPU-cycle equality is not a C64U timing
 * certification; preprocessing, REU traffic and raster work need measurement.
 */
void havoc_raster_work(void);

#endif
