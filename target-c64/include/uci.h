#ifndef HAVOC_UCI_H
#define HAVOC_UCI_H
#include <stdint.h>
enum { UCI_OK=0, UCI_ABSENT=1, UCI_PROTOCOL=2, UCI_OVERFLOW=3, UCI_TIMEOUT=4 };
extern volatile uint8_t uci_error,uci_busy,uci_complete,uci_packet_ready;
extern uint8_t uci_status[32],uci_status_length,uci_packet_last;
extern uint16_t uci_response_length;
/* Buffers remain alive until busy clears. Timeout is NOT cancellation: keep
 * any in-flight REU destination pinned. stream=1 pauses on every packet. */
/* This client deliberately caps commands at112 bytes (its longest path). */
uint8_t uci_start(const uint8_t *command,uint16_t length,
                  uint8_t *response,uint16_t capacity,uint8_t stream);
void uci_step(void); /* <=16 command bytes or 32 reply bytes */
void uci_accept_packet(void);
uint8_t uci_status_ok(void);
uint8_t uci_transfer_matches(uint32_t count,uint32_t destination);
#endif
