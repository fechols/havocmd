; Pure foreground calculations in RAM underneath I/O. IRQs are disabled by
; startup.S. Gates preserve every argument/return register except CPU flags.
; Banked callees must not access MMIO, and must call each other directly.
.macro BANK_GATE name, target, region=.text
.section \region\().\name,"ax",@progbits
.globl \name
.type \name,@function
\name:
    pha
    lda #$34
    sta $01
    pla
    jsr \target
    pha
    lda #$35
    sta $01
    pla
    rts
.size \name, .-\name
.endm

BANK_GATE pano_picture, pano_picture_banked
BANK_GATE controller_adjacent, controller_adjacent_banked
BANK_GATE pano_retain, pano_retain_banked
BANK_GATE pano_strip, pano_strip_banked, .highcode
