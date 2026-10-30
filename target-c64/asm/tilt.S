; Level circular renderer. Select this INSTEAD OF raster.S when linking.
; $ED/$EE hold the pixel entry pointer; $EF is a per-line pair count.
.zeropage __rc2
.zeropage __rc3
.section .data.havoc_tilt,"aw",@progbits
.globl havoc_raster_x,havoc_raster_y,havoc_raster_top,havoc_raster_delay
.globl havoc_raster_late_lines,havoc_raster_frames,havoc_tilt_k
havoc_raster_x: .byte 0
havoc_raster_y: .byte 0
havoc_raster_top: .byte 31
havoc_raster_delay: .byte 13
havoc_raster_late_lines: .byte 0
havoc_raster_frames: .byte 0
havoc_tilt_k: .byte 0
.globl havoc_tilt_row_x,havoc_tilt_row_y
havoc_tilt_row_x:
.Lrow_x: .byte 0
havoc_tilt_row_y:
.Lrow_y: .byte 0
.Lrows: .byte 0
.Llast_line: .byte 0

.section .bss.havoc_nibbles,"aw",@nobits
.p2align 8
.globl havoc_raster_nibbles
havoc_raster_nibbles: .space 256

.section .text.havoc_tilt,"ax",@progbits
.globl havoc_raster_init,havoc_raster_prepare,havoc_raster_frame
.globl havoc_tilt_line,havoc_tilt_row_setup
.section .disk_init,"ax",@progbits
havoc_raster_init:
    sei
    cld
    lda #$35
    sta $01
    lda #0
    sta $d011
    sta $d015
    sta $d01a
    sta havoc_raster_late_lines
    sta havoc_raster_frames
    sta havoc_tilt_k
    lda #$8f
    sta $d031
    ldx #0
.Lnibble_loop:
    txa
    lsr
    lsr
    lsr
    lsr
    sta havoc_raster_nibbles,x
    inx
    bne .Lnibble_loop
    rts

.section .text.havoc_tilt,"ax",@progbits
; Source operands and all pixel step slots remain fixed for level scrolling.
; Keep the exported debug angle zero even if RAM was dirty before startup.
havoc_raster_prepare:
    lda #0
    sta havoc_tilt_k
    lda havoc_raster_x
    sta .Lrow_x
    lda havoc_raster_y
    sta .Lrow_y
    lda havoc_raster_delay
    sta .Ldelay+1
    rts

; Exactly166 cycles through RTS; the caller's JSR makes172.
.p2align 8
havoc_tilt_row_setup:
    lda .Lrow_x
    and #254
    tax
    lda havoc_tilt_entries,x
    sta $ed
    lda havoc_tilt_entries+1,x
    sta $ee
    lda .Lrow_x
    and #7
    asl
    tax
    lda .Lphase_pointers,x
    sta .Lphase_dispatch+1
    lda .Lphase_pointers+1,x
    sta .Lphase_dispatch+2
    ; The setup above takes48 cycles. This local loop adds106, two BITs
    ; add6, and RTS adds6. Page alignment keeps the loop branch constant.
    ldx #21
.Lrow_padding:
    dex
    bne .Lrow_padding
    bit $ef
    bit $ef
    rts

.p2align 7
havoc_tilt_line:
    jsr havoc_raster_work
    lda #101
    sta $ef
    jsr havoc_tilt_row_setup
    ldy .Lrow_y
.Ldelay:
    ldx #13
.Ldelay_loop:
    dex
    bne .Ldelay_loop
.Lwrap_pad:
    nop
    nop
    nop
.Lwrap_pad_end:
.Lphase_dispatch:
    jmp .Lphase_even
.Lphase_even:
    .rept 13
    nop
    .endr
    jmp ($00ed)
.Lphase_odd:
    .rept 5
    nop
    .endr
    jmp ($00ed)
.p2align 4
.Lphase_pointers:
    .short .Lphase_even,.Lphase_odd,.Lphase_even,.Lphase_odd
    .short .Lphase_even,.Lphase_odd,.Lphase_even,.Lphase_odd

havoc_raster_frame:
    sei
    jsr havoc_raster_prepare
.Lwait_top:
    lda $d011
    bmi .Lwait_top
    lda havoc_raster_top
    cmp $d012
    bne .Lwait_top
    sta .Llast_line
    lda #240
    sta .Lrows
    jmp .Ldraw_line
.Ldraw_line:
    lda #$80
    ldx #$8f
    sta $d031
    stx $d031
    jsr havoc_tilt_line
    lda $d012
    cmp .Llast_line
    beq .Lon_time
    inc havoc_raster_late_lines
.Lon_time:
    lda .Llast_line
    beq .Lrestore_delay
    cmp #6
    bne .Ldelay_ready
    lda $d011
    bpl .Ldelay_ready
    lda havoc_raster_delay
    sec
    sbc #12
    bcc .Lminimum_delay
    bne .Lwrap_delay
.Lminimum_delay:
    lda #1
.Lwrap_delay:
    sta .Ldelay+1
    lda #$4c
    sta .Lwrap_pad
    lda #<.Lwrap_pad_end
    sta .Lwrap_pad+1
    lda #>.Lwrap_pad_end
    sta .Lwrap_pad+2
    jmp .Ldelay_ready
.Lrestore_delay:
    lda havoc_raster_delay
    sta .Ldelay+1
    lda #$ea
    sta .Lwrap_pad
    sta .Lwrap_pad+1
    sta .Lwrap_pad+2
.Ldelay_ready:
    inc .Lrow_y
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

#include "tilt_pixels.inc"
