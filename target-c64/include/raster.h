#ifndef HAVOC_RASTER_H
#define HAVOC_RASTER_H

#include <stdint.h>

#define HAVOC_RASTER_WIDTH 200u
#define HAVOC_RASTER_HEIGHT 240u
#define HAVOC_CACHE_WIDTH 256u
#define HAVOC_CACHE_HEIGHT 256u
#define HAVOC_CACHE_ADDRESS 0x4000u
#define HAVOC_CACHE_BYTES 0x8000u
#define HAVOC_RASTER_WORK_CYCLES 256u

/* Byte column c is at $4000 + 256*c. Low nibble is logical x=2*c;
 * high nibble is x=2*c+1. The cache wraps in both axes. ROMs must be out.
 * Root linker must exclude $4000-$BFFF from code, data and software stack. */
#define HAVOC_RASTER_CACHE ((volatile uint8_t *)HAVOC_CACHE_ADDRESS)

extern volatile uint8_t havoc_raster_x;
extern volatile uint8_t havoc_raster_y;
/* NTSC prototype calibration: first line31, including the raster-zero wrap.
 * The production DDA renderer still requires its own physical acceptance. */
extern volatile uint8_t havoc_raster_top;
/* Initial13. Each increment moves all stores five turbo cycles later.
 * Keep within [1,24] for calibration; reducing it enlarges the end margin. */
extern volatile uint8_t havoc_raster_delay;
extern volatile uint8_t havoc_raster_late_lines;
extern volatile uint8_t havoc_raster_frames;
extern volatile int8_t havoc_tilt_k;
/* Tick once outside the picture. Nonzero requests a smooth return to level. */
void havoc_tilt_tick(uint8_t flatten);

/* Init disables IRQ sources, banks ROMs out, selects $D031=$8F, fills the
 * cache with a diagnostic grid, and builds a page-aligned nibble table.
 * Call only after the machine/turbo preflight. Does not return to BASIC. */
void havoc_raster_init(void);

/* Patch source addresses for x, then render one frame. IRQs remain disabled.
 * Returns immediately after the last active row; caller owns vertical blank.
 * x/y must change only outside this function. No C callbacks during drawing. */
void havoc_raster_frame(void);

/* Assembly-only integration hook, called once per active row before pixels.
 * EXACTLY 256 nominal CPU cycles INCLUDING RTS, on every branch. JSR and
 * Y preservation are outside that budget. May clobber A/X/Y. The linked audio
 * kernel supplies this hook. REU DMA/bus stalls are NOT ordinary CPU cycles: a hook
 * performing them needs an independently measured wall-time contract. */
void havoc_raster_work(void);

#endif
