; Run before the CRT touches zero page, screen RAM or KERNAL RAM vectors.
; This program owns the machine until reset; it never returns to BASIC.
.section .init.000,"ax",@progbits
.globl havoc_boot_begin
havoc_boot_begin:
    sei
    cld
    ldx #$2f
    stx $00
    ldx #$37
    stx $01
    lda #$7f
    sta $dc0d
    sta $dd0d
    lda $dc0d
    lda $dd0d
    lda #0
    sta $d01a
    sta $d011
    sta $d015
    lda #$ff
    sta $d019
    ; ROM is still mapped; stores install safe RAM vectors underneath it.
    lda #<havoc_ignore_interrupt
    sta $fffa
    sta $fffe
    lda #>havoc_ignore_interrupt
    sta $fffb
    sta $ffff
    lda #$35
    sta $01
    lda #0
    sta $d020

.section .text.havoc_boot,"ax",@progbits
havoc_ignore_interrupt:
    rti
