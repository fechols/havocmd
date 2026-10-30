; HAVOC NTSC border renderer. Original implementation from timing principles.
; $D011 display disabled: every painted dot comes from $D020, including edges.
; This prototype deliberately needs on-machine horizontal/vertical calibration.
.zeropage __rc2
.zeropage __rc3
.zeropage __rc4
.zeropage __rc5

.section .data.havoc_raster,"aw",@progbits
.globl havoc_raster_x, havoc_raster_y, havoc_raster_top, havoc_raster_delay
.globl havoc_raster_late_lines, havoc_raster_frames
havoc_raster_x: .byte 0
havoc_raster_y: .byte 0
; Measured on C64U firmware 1.1.0/core 1.49, NTSC, 64 MHz.
havoc_raster_top: .byte 31
havoc_raster_delay: .byte 13
havoc_raster_late_lines: .byte 0
havoc_raster_frames: .byte 0
.Lrows: .byte 0
.Llast_line: .byte 0
.Lfirst_column: .byte 0

.section .bss.havoc_nibbles,"aw",@nobits
.p2align 8
.globl havoc_raster_nibbles
havoc_raster_nibbles:
    .space 256

.section .text.havoc_raster,"ax",@progbits
.globl havoc_raster_init
havoc_raster_init:
    sei
    cld
    lda #$35
    sta $01
    lda #$7f
    sta $dc0d
    sta $dd0d
    lda $dc0d
    lda $dd0d
    lda #0
    sta $d01a
    sta $d015
    sta $d011
    sta havoc_raster_late_lines
    sta havoc_raster_frames
    lda #$8f
    sta $d031
    ldx #0
.Linit_nibbles:
    txa
    lsr
    lsr
    lsr
    lsr
    sta havoc_raster_nibbles,x
    inx
    bne .Linit_nibbles
#ifdef HAVOC_PRODUCTION
    rts
#else
    lda #0
    sta __rc2
    sta __rc4
    lda #$40
    sta __rc3
.Linit_column:
    ldy #0
.Linit_row:
    ; Sixteen-color checker/grid: single rows and column boundaries make
    ; both scrolling axes and even/odd nibble ordering visibly testable.
    tya
    lsr
    lsr
    lsr
    lsr
    clc
    adc __rc4
    and #15
    sta __rc5
    eor #15
    asl
    asl
    asl
    asl
    ora __rc5
    cpy #0
    bne .Lnot_top_edge
    lda #$11
.Lnot_top_edge:
    sta (__rc2),y
    iny
    bne .Linit_row
    inc __rc4
    inc __rc3
    lda __rc3
    cmp #$c0
    bne .Linit_column
    rts
#endif

; Called before beam synchronization. All generated fetches use a page
; base and indexed Y, so every source load takes exactly four CPU cycles.
.section .text.havoc_raster,"ax",@progbits
.globl havoc_raster_prepare
havoc_raster_prepare:
.Lprepare:
    lda havoc_raster_delay
    sta .Leven_delay+1
    sta .Lodd_delay+1
    lda havoc_raster_x
    lsr
    sta .Lfirst_column
    lda havoc_raster_x
    and #1
    bne .Lprepare_odd
    lda #<(.Leven_line)
    sta .Lrender_call+1
    lda #>(.Leven_line)
    sta .Lrender_call+2
    ldx #0
.Lpatch_even:
    lda .Leven_operands,x
    sta __rc2
    lda .Leven_operands+1,x
    sta __rc3
    txa
    lsr
    clc
    adc .Lfirst_column
    and #127
    clc
    adc #$40
    ldy #0
    sta (__rc2),y
    inx
    inx
    cpx #200
    bne .Lpatch_even
    rts
.Lprepare_odd:
    lda #<(.Lodd_line)
    sta .Lrender_call+1
    lda #>(.Lodd_line)
    sta .Lrender_call+2
    ldx #0
.Lpatch_odd:
    lda .Lodd_operands,x
    sta __rc2
    lda .Lodd_operands+1,x
    sta __rc3
    txa
    lsr
    clc
    adc .Lfirst_column
    and #127
    clc
    adc #$40
    ldy #0
    sta (__rc2),y
    inx
    inx
    cpx #202
    bne .Lpatch_odd
    rts

.globl havoc_raster_frame
havoc_raster_frame:
    sei
    jsr .Lprepare
    ; NTSC's visible picture wraps through raster zero (31..262,0..7).
    ; Poll the next target directly. A forced bottom/wrap wait would skip a
    ; refresh after a picture that already finished on the next frame.
.Lwait_top:
    lda $d011
    bmi .Lwait_top
    lda havoc_raster_top
    cmp $d012
    bne .Lwait_top
    sta .Llast_line
    lda #240
    sta .Lrows
    ldy havoc_raster_y
    jmp .Ldraw_line
.p2align 8
.Ldraw_line:
    ; Return briefly to one-MHz timing, then turbo, to reset horizontal phase.
    lda #$80
    ldx #$8f
    sta $d031
    stx $d031
.Lrender_call:
    jsr .Leven_line
    lda $d012
    cmp .Llast_line
    beq .Lon_time
    inc havoc_raster_late_lines
.Lon_time:
    ; On this NTSC core the raster-zero transition is observed one PHI2 later
    ; relative to visible dots. Shorten that line's preamble, then restore it.
    ; Patch ahead of the next line wait so ordinary line synchronization stays
    ; unchanged. The low-byte value 6 is also line 6, so check raster bit 8.
    lda .Llast_line
    beq .Lrestore_delay
    cmp #6
    bne .Ldelay_ready
    lda $d011
    bpl .Ldelay_ready
    lda havoc_raster_delay
    sec
    sbc #12
    bcc .Lminimum_wrap_delay
    bne .Lwrap_delay
.Lminimum_wrap_delay:
    lda #1
.Lwrap_delay:
    sta .Leven_delay+1
    sta .Lodd_delay+1
    ; Three NOPs take6 cycles; JMP past the same3 bytes takes3. Together
    ; with12 fewer delay iterations this advances raster0 by exactly63.
    lda #$4c
    sta .Leven_pad
    sta .Lodd_pad
    lda #<.Leven_pad_end
    sta .Leven_pad+1
    lda #>.Leven_pad_end
    sta .Leven_pad+2
    lda #<.Lodd_pad_end
    sta .Lodd_pad+1
    lda #>.Lodd_pad_end
    sta .Lodd_pad+2
    jmp .Ldelay_ready
.Lrestore_delay:
    lda havoc_raster_delay
    sta .Leven_delay+1
    sta .Lodd_delay+1
    lda #$ea
    sta .Leven_pad
    sta .Leven_pad+1
    sta .Leven_pad+2
    sta .Lodd_pad
    sta .Lodd_pad+1
    sta .Lodd_pad+2
.Ldelay_ready:
    iny
    dec .Lrows
    beq .Lframe_done
    lda .Llast_line
.Lwait_next:
    cmp $d012
    beq .Lwait_next
    lda $d012
    sta .Llast_line
    jmp .Ldraw_line
.Lframe_done:
    inc havoc_raster_frames
    rts

; The linked audio kernel supplies the mandatory exact-cycle work hook.

#include "raster_pixels.inc"
