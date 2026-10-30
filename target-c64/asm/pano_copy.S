; SPDX-License-Identifier: MIT
; Fixed-shape foreground panorama copies. IRQs are disabled by startup.
; LLVM-MOS C ABI: destination rc2/3, source rc4/5, count A/X.
; All functions preserve every imaginary register and the software stack;
; A/X/Y and arithmetic flags are caller-clobbered. Counts are contracts, not
; runtime loop bounds for band/row; strip accepts1..8. Writable RAM is required.
.zeropage __rc2
.zeropage __rc3
.zeropage __rc4
.zeropage __rc5

; count=64, source=$E6C0. Destination is $E700 or $EB00 plus column0..127.
; Source[c*8+r] goes to band_base+r*128+((column+c)&127), c/r=0..7.
; Fixed source addressing stays within $E6C0..$E6FF; there are no page-cross
; load penalties. The128-byte alignment keeps the loop branch on one page.
.section .text.pano_copy_band,"ax",@progbits
.p2align 7
.globl pano_copy_band
.type pano_copy_band,@function
pano_copy_band:
    lda __rc3
    sta .Lband_row0+2
    sta .Lband_row1+2
    clc
    adc #1
    sta .Lband_row2+2
    sta .Lband_row3+2
    adc #1
    sta .Lband_row4+2
    sta .Lband_row5+2
    adc #1
    sta .Lband_row6+2
    sta .Lband_row7+2
    ldx #0
    ldy __rc2
.Lband_column:
    lda $e6c0,x
.Lband_row0: sta $e700,y
    lda $e6c1,x
.Lband_row1: sta $e780,y
    lda $e6c2,x
.Lband_row2: sta $e800,y
    lda $e6c3,x
.Lband_row3: sta $e880,y
    lda $e6c4,x
.Lband_row4: sta $e900,y
    lda $e6c5,x
.Lband_row5: sta $e980,y
    lda $e6c6,x
.Lband_row6: sta $ea00,y
    lda $e6c7,x
.Lband_row7: sta $ea80,y
    txa
    clc
    adc #8
    tax
    iny
    tya
    and #127
    tay
    cpx #64
    bne .Lband_column
    rts
.size pano_copy_band, .-pano_copy_band

; count=128, destination=$4000+physical_row, source is one128-byte band row
; at $E700/$EB00 + n*128, n=0..7. Source[c] goes to $4000+c*256+row.
; The last store uses page$BF; the patched high byte reaches$C0 only AFTER
; that store. Source pointers and the passed destination remain unchanged.
.section .text.pano_copy_row,"ax",@progbits
.p2align 5
.globl pano_copy_row
.type pano_copy_row,@function
pano_copy_row:
    lda __rc3
    sta .Lrow_store+2
    ldx __rc2
    ldy #0
.Lrow_column:
    lda (__rc4),y
.Lrow_store:
    sta $4000,x
    inc .Lrow_store+2
    iny
    bpl .Lrow_column
    rts
.size pano_copy_row, .-pano_copy_row

; count=1..8, source=$E6C0+first with first+count<=8. Destination is
; $4000..$BFFF at one physical row. Copy eight columns, source stride8;
; destination rows wrap255->0 within each page, pages wrap$BF->$40.
; Unrolling columns lets each row use eight fixed source/store pairs.
.section .highcode.pano_copy_strip,"ax",@progbits
.globl pano_copy_strip
.type pano_copy_strip,@function
pano_copy_strip:
    sta .Lstrip_count+1
    lda __rc3
    sta .Lstrip_store0+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page1
    lda #$40
.Lstrip_page1:
    sta .Lstrip_store1+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page2
    lda #$40
.Lstrip_page2:
    sta .Lstrip_store2+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page3
    lda #$40
.Lstrip_page3:
    sta .Lstrip_store3+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page4
    lda #$40
.Lstrip_page4:
    sta .Lstrip_store4+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page5
    lda #$40
.Lstrip_page5:
    sta .Lstrip_store5+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page6
    lda #$40
.Lstrip_page6:
    sta .Lstrip_store6+2
    clc
    adc #1
    cmp #$c0
    bne .Lstrip_page7
    lda #$40
.Lstrip_page7:
    sta .Lstrip_store7+2
    lda __rc4
    clc
    sta .Lstrip_source0+1
    adc #8
    sta .Lstrip_source1+1
    adc #8
    sta .Lstrip_source2+1
    adc #8
    sta .Lstrip_source3+1
    adc #8
    sta .Lstrip_source4+1
    adc #8
    sta .Lstrip_source5+1
    adc #8
    sta .Lstrip_source6+1
    adc #8
    sta .Lstrip_source7+1
    ldx #0
    ldy __rc2
.Lstrip_row:
.Lstrip_source0: lda $e6c0,x
.Lstrip_store0: sta $4000,y
.Lstrip_source1: lda $e6c8,x
.Lstrip_store1: sta $4000,y
.Lstrip_source2: lda $e6d0,x
.Lstrip_store2: sta $4000,y
.Lstrip_source3: lda $e6d8,x
.Lstrip_store3: sta $4000,y
.Lstrip_source4: lda $e6e0,x
.Lstrip_store4: sta $4000,y
.Lstrip_source5: lda $e6e8,x
.Lstrip_store5: sta $4000,y
.Lstrip_source6: lda $e6f0,x
.Lstrip_store6: sta $4000,y
.Lstrip_source7: lda $e6f8,x
.Lstrip_store7: sta $4000,y
    inx
    iny
.Lstrip_count:
    cpx #8
    bne .Lstrip_row
    rts
.size pano_copy_strip, .-pano_copy_strip
