#ifndef HAVOC_TILT_H
#define HAVOC_TILT_H
#include <stdint.h>
/* Signed K in [-10,10], roughly +/-1.4 degrees. A frame samples no more than
 * two extra X pixels and four extra Y rows on either side of its viewport.
 * Return K to zero before starting a panorama prefetch/slide. */
extern volatile int8_t havoc_tilt_k;
/* Once per frame, outside rendering. Nonzero returns toward level; wait for
 * K==0 before panorama prefetch, and keep its margin guard until then. */
void havoc_tilt_tick(uint8_t flatten);
#endif
