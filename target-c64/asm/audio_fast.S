; Foreground helpers only. Official6502 instructions; the256-cycle active-line
; decoder is unchanged. Imaginary registers rc2..rc4 are caller-saved by MOS C.
.zeropage __rc2
.zeropage __rc3
.zeropage __rc4
.section .lowcode.audio_timer16,"ax",@progbits
.globl audio_timer16
audio_timer16:
.Ltimer_again:
    ldx $dd05
    lda $dd04
    cpx $dd05
    bne .Ltimer_again
    rts
.section .lowcode.audio_unpack_fast,"ax",@progbits
.globl audio_unpack_fast
audio_unpack_fast:
    sta .Lcode0+2
    sta .Lcode1+2
    sta .Lcode2+2
    sta .Lcode3+2
    stx __rc2
    ldx #0
    ldy #0
.Lunpack_group:
    lda $e600,x
    sta __rc3
    and #63
.Lcode0:
    sta $c000,y
    lda $e601,x
    sta __rc4
    ; Carry injects b0's high two bits into b1 without six accumulator shifts.
    asl __rc3
    rol
    asl __rc3
    rol
    and #63
.Lcode1:
    sta $c002,y
    lda __rc4
    lsr
    lsr
    lsr
    lsr
    sta __rc4
    lda $e602,x
    asl
    asl
    asl
    asl
    ora __rc4
    and #63
.Lcode2:
    sta $c004,y
    lda $e602,x
    lsr
    lsr
.Lcode3:
    sta $c006,y
    inx
    inx
    inx
    tya
    clc
    adc #8
    tay
    cpx __rc2
    bne .Lunpack_group
    rts

.section .highcode.audio_lut_fast,"ax",@progbits
.globl audio_lut_fast
audio_lut_fast:
    sta .Lll0+2
    sta .Lll1+2
    sta .Lll2+2
    sta .Lll3+2
    sta .Lll4+2
    sta .Lll5+2
    sta .Lll6+2
    sta .Lll7+2
    sta .Llh0+2
    sta .Llh1+2
    sta .Llh2+2
    sta .Llh3+2
    sta .Llh4+2
    sta .Llh5+2
    sta .Llh6+2
    sta .Llh7+2
    sta .Lrl0+2
    sta .Lrl1+2
    sta .Lrl2+2
    sta .Lrl3+2
    sta .Lrl4+2
    sta .Lrl5+2
    sta .Lrl6+2
    sta .Lrl7+2
    sta .Lrh0+2
    sta .Lrh1+2
    sta .Lrh2+2
    sta .Lrh3+2
    sta .Lrh4+2
    sta .Lrh5+2
    sta .Lrh6+2
    sta .Lrh7+2
    ldx #0
    ldy #0
.Lleft_values:
    lda $e680,y
.Lll0: sta $c200,x
.Lll1: sta $c208,x
.Lll2: sta $c210,x
.Lll3: sta $c218,x
.Lll4: sta $c220,x
.Lll5: sta $c228,x
.Lll6: sta $c230,x
.Lll7: sta $c238,x
    lda $e681,y
.Llh0: sta $c240,x
.Llh1: sta $c248,x
.Llh2: sta $c250,x
.Llh3: sta $c258,x
.Llh4: sta $c260,x
.Llh5: sta $c268,x
.Llh6: sta $c270,x
.Llh7: sta $c278,x
    iny
    iny
    inx
    cpx #8
    bne .Lleft_values
    ldx #0
    ldy #0
.Lright_values:
    lda $e690,x
.Lrl0: sta $c280,y
.Lrl1: sta $c281,y
.Lrl2: sta $c282,y
.Lrl3: sta $c283,y
.Lrl4: sta $c284,y
.Lrl5: sta $c285,y
.Lrl6: sta $c286,y
.Lrl7: sta $c287,y
    lda $e691,x
.Lrh0: sta $c2c0,y
.Lrh1: sta $c2c1,y
.Lrh2: sta $c2c2,y
.Lrh3: sta $c2c3,y
.Lrh4: sta $c2c4,y
.Lrh5: sta $c2c5,y
.Lrh6: sta $c2c6,y
.Lrh7: sta $c2c7,y
    inx
    inx
    tya
    clc
    adc #8
    tay
    cpx #16
    bne .Lright_values
    rts
