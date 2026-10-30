; Coherent CIA2 A/B down-counter read. LLVM-MOS uint32 return is
; A (bits0..7), X (8..15), __rc2 (16..23), __rc3 (24..31).
; Read counters are live: this does not assume a low-byte read latches high.
; https://www.retrodocs.fr/wp-content/uploads/pdf/MOS-6526.pdf
;
; Upper-byte rereads reject byte/cascade rollover during the snapshot. The
; $FFFC..$FFFF exclusion avoids the immediate A-reload window as well; the
; local oracle exercises B carry propagation delays of zero through3 PHI2.
; That delay model is conservative test coverage, not measured C64U behavior.
; No timer/control/interrupt registers are written. Y is caller-clobbered.
.zeropage __rc2
.zeropage __rc3
.section .midcode.timer_read,"ax",@progbits
.globl timer_read
.type timer_read,@function
timer_read:
.Lretry:
    lda $dd07
    sta __rc3
    lda $dd06
    sta __rc2
    ldx $dd05
    ldy $dd04
    cpx $dd05
    bne .Lretry
    lda $dd06
    cmp __rc2
    bne .Lretry
    lda $dd07
    cmp __rc3
    bne .Lretry
    cpx #$ff
    bne .Lstable
    cpy #$fc
    bcs .Lretry
.Lstable:
    tya
    rts
.size timer_read,.-timer_read
