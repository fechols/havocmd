#ifndef HAVOC_PLATFORM_H
#define HAVOC_PLATFORM_H
#include <stdint.h>
#ifndef AUDIO_SAMPLE_RATE
#ifdef HAVOC_PRODUCTION
#define AUDIO_SAMPLE_RATE 16000UL
#else
#define AUDIO_SAMPLE_RATE 32000UL
#endif
#endif
#define REG(a) (*(volatile uint8_t *)(a))
#define RING_ADDRESS 0xfe0000UL
#define RING_BYTES 65536UL
void reu_fetch(uint32_t source, void *destination, uint16_t count);
void reu_stash(const void *source, uint32_t destination, uint16_t count);
void sampler_stop(void);
void sampler_start(uint16_t divider);
void sampler_prepare(uint16_t divider);
void sampler_arm(void);
void platform_init(void);
uint32_t timer_read(void);
uint16_t raster_line(void);
uint16_t sampler_calibrate(void);
void probe_dma(void);
extern volatile uint16_t havoc_dma8_budget_ticks,havoc_dma16_budget_ticks;
extern volatile uint32_t sampler_ticks_per_ring;
#endif
