; SPDX-License-Identifier: MIT
; Copyright (c) 2026 Fredric Echols
; Adapted from sueme64/target-c64/asm/memcpy.S (same MIT license).
;
; LLVM-MOS C ABI: destination rc2/3, source rc4/5, count A/X.
; Return the original destination in rc2/3. rc4..rc8, A/X/Y are
; caller-clobbered. No software-stack frame or compiler static scratch.
; The ranges must not overlap, as required by memcpy.
.zeropage __rc2
.zeropage __rc3
.zeropage __rc4
.zeropage __rc5
.zeropage __rc6
.zeropage __rc7
.zeropage __rc8

.section .text.memcpy,"ax",@progbits
.globl memcpy
.type memcpy,@function
memcpy:
    sta __rc8
    lda __rc2
    sta __rc6
    lda __rc3
    sta __rc7
    ldy #0
    cpx #0
    beq .Lmemcpy_tail
.Lmemcpy_page:
    lda (__rc4),y
    sta (__rc6),y
    iny
    bne .Lmemcpy_page
    inc __rc5
    inc __rc7
    dex
    bne .Lmemcpy_page
.Lmemcpy_tail:
    cpy __rc8
    beq .Lmemcpy_done
    lda (__rc4),y
    sta (__rc6),y
    iny
    jmp .Lmemcpy_tail
.Lmemcpy_done:
    rts
.size memcpy, .-memcpy

; Rust sees an opaque foreign call with an escaped RAM pointer (rc2/3).
; This models the synchronous REU operation's ordinary-memory side effects;
; it must not be replaced with a Rust volatile write to the command register.
; LLVM-MOS C ABI: command in A, RAM pointer in rc2/3. Only A is read.
.section .text.reu_execute,"ax",@progbits
.globl reu_execute
.type reu_execute,@function
reu_execute:
    sta $df01
    rts
.size reu_execute, .-reu_execute
