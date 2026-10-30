.section .text
.globl main,havoc_raster_work
main:
    rts
havoc_raster_work:
    .rept 125
    nop
    .endr
    rts
