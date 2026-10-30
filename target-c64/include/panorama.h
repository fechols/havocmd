#ifndef HAVOC_PANORAMA_H
#define HAVOC_PANORAMA_H
#include <stdint.h>

/* v2: 32768-byte seed, then X-major 16x8/64-byte tiles. Coordinates below
 * describe the unrotated 200x240 read footprint. No writes occur in raster. */
extern volatile uint16_t pano_camera_x,pano_camera_y;
extern volatile uint16_t pano_max_job_ticks;
extern volatile uint8_t pano_prefetched,pano_retired;
uint8_t pano_init(uint32_t address,uint16_t width,uint16_t height);
void pano_request(int8_t dx,int8_t dy);
void pano_tick(void);
/* CIA is a down-counter. Caller reserves its raster preparation guard before
 * deadline. Returns the number of completed bounded transfer/copy jobs. */
uint8_t pano_step(uint8_t max_jobs,uint32_t deadline);
/* Begin shortly before the audible song boundary: freezes the current camera
 * while filling hidden columns. Returns zero if another transition owns them. */
uint8_t pano_prefetch(uint32_t address,uint16_t width,uint16_t height);
uint8_t pano_can_transition(void);
void pano_transition(void);
uint8_t pano_transition_done(void);
uint8_t pano_busy(void);
/* Enable before a nonzero tilt; disable only once the renderer is level.
 * Failure leaves the existing guard intact and asks the caller to flatten. */
uint8_t pano_tilt_guard(uint8_t enabled);
#endif
