; Fixed-cycle DLT5 publication kernel. See include/audio_step.h for producer ABI.
; Predictor saturation and signed8 rounding each take a padded, exact path.
; The four lookup arrays share one page. X is a combined six-bit stereo code.

.section .text.havoc_audio,"ax",@progbits

.macro AUDIO_SAMPLE name, low, high, lutlo, luthi
    clc
    lda \low
.Ldelta_lo_\name:
    adc 0xc200+\lutlo,x
    sta \low
    lda \high
.Ldelta_hi_\name:
    adc 0xc200+\luthi,x
    bvs .Loverflow_\name
    sta \high
    lda \low
    cmp #128
    lda \high
    adc #0
    bvs .Lclip_\name
    nop
    jmp .Lpublish_\name
.Lclip_\name:
    lda #127
    nop
.Lpublish_\name:
.Lpcm_normal_\name:
    sta 0xc100,y
    bit 0xf5
    nop
    jmp .Lsample_done_\name
.Loverflow_\name:
    bmi .Lpositive_\name
    lda #0
    sta \low
    lda #128
    sta \high
.Lpcm_negative_\name:
    sta 0xc100,y
    .rept 6
    nop
    .endr
    jmp .Lsample_done_\name
.Lpositive_\name:
    lda #255
    sta \low
    lda #127
    sta \high
.Lpcm_positive_\name:
    sta 0xc100,y
    .rept 7
    nop
    .endr
.Lsample_done_\name:
.endm

.p2align 8
.globl havoc_raster_work
.type havoc_raster_work,@function
havoc_raster_work:
    dec 0xf5
    bpl .Lactive
    jmp .Lhandoff
.Lactive:
    ldy 0xf4
.Lcodes_first:
    ldx 0xc000,y
    AUDIO_SAMPLE left_first, 0xf0, 0xf1, 0, 64
    iny
    AUDIO_SAMPLE right_first, 0xf2, 0xf3, 128, 192
    iny
.Lcodes_second:
    ldx 0xc000,y
    AUDIO_SAMPLE left_second, 0xf0, 0xf1, 0, 64
    iny
    AUDIO_SAMPLE right_second, 0xf2, 0xf3, 128, 192
    iny
    sty 0xf4
    nop
    nop
    rts
.size havoc_raster_work,.-havoc_raster_work

; This page boundary fixes branch timing independently of final link placement.
.p2align 8
.Lhandoff:
    lda #0
    sta 0xf5
    ldx 0xf6
    lda havoc_audio_chunk_state,x
    cmp #2
    bne .Lold_not_playing
    lda #3
    sta havoc_audio_chunk_state,x
    jmp .Lold_done
.Lold_not_playing:
    bit 0xf5
    nop
    nop
    nop
.Lold_done:
    lda .Lnext_chunk,x
    tax
    lda havoc_audio_chunk_state,x
    cmp #1
    bne .Lno_ready
    stx 0xf6
    lda #2
    sta havoc_audio_chunk_state,x
    lda havoc_audio_chunk_units,x
    sta 0xf5
    lda #0
    sta 0xf4
    lda havoc_audio_chunk_reset,x
    beq .Lno_reset
    lda #0
    sta 0xf0
    sta 0xf1
    sta 0xf2
    sta 0xf3
    jmp .Lreset_done
.Lno_reset:
    .rept 8
    nop
    .endr
.Lreset_done:
    lda havoc_audio_codes_hi,x
    sta .Lcodes_first+2
    sta .Lcodes_second+2
    lda havoc_audio_pcm_hi,x
    sta .Lpcm_normal_left_first+2
    sta .Lpcm_negative_left_first+2
    sta .Lpcm_positive_left_first+2
    sta .Lpcm_normal_right_first+2
    sta .Lpcm_negative_right_first+2
    sta .Lpcm_positive_right_first+2
    sta .Lpcm_normal_left_second+2
    sta .Lpcm_negative_left_second+2
    sta .Lpcm_positive_left_second+2
    sta .Lpcm_normal_right_second+2
    sta .Lpcm_negative_right_second+2
    sta .Lpcm_positive_right_second+2
    lda havoc_audio_lut_hi,x
    sta .Ldelta_lo_left_first+2
    sta .Ldelta_hi_left_first+2
    sta .Ldelta_lo_right_first+2
    sta .Ldelta_hi_right_first+2
    sta .Ldelta_lo_left_second+2
    sta .Ldelta_hi_left_second+2
    sta .Ldelta_lo_right_second+2
    sta .Ldelta_hi_right_second+2
    jmp .Lready_delay
.Lno_ready:
    .rept 98
    nop
    .endr
    bit 0xf5
    rts
.Lready_delay:
    .rept 26
    nop
    .endr
    rts
.Lnext_chunk:
    .byte 1,2,3,4,5,0
