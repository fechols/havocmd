/* One PRG. Startup overlays in $4000-$BFFF retire before the panorama cache. */
__basic_zp_start = 0x0002;
__basic_zp_end = 0x0090;
__rc0 = 0x0002;
INCLUDE imag-regs.ld
MEMORY {
    ram (rwx) : ORIGIN = 0x0801, LENGTH = 0x37FF
    low (rw) : ORIGIN = 0x0200, LENGTH = 0x0600
    zp (rw) : ORIGIN = 0x0022, LENGTH = 0x006E
    initializers (r) : ORIGIN = 0x4000, LENGTH = 0x3000
    bootwork (rx) : ORIGIN = 0x7000, LENGTH = 0x5000
    high (rx) : ORIGIN = 0xEF00, LENGTH = 0x10FA
    mid (rx) : ORIGIN = 0xCC00, LENGTH = 0x0380
    bankram (rx) : ORIGIN = 0xD000, LENGTH = 0x1000
}
INPUT(basic-header.o)
/* C64's default CRT assumes .data is already at its run address. Our low-RAM
 * code/data are loaded at $4000 and must be relocated before main. */
INPUT(libcopy-data.a)
EXTERN(__do_copy_data)
/* Rust IR lowers these operations after archive discovery. Keep the MOS CRT
 * implementations available without pulling Rust's generic memory runtime. */
EXTERN(__mulhi3 __mulsi3 __udivsi3 __lshrqi3 __ashlqi3)
REGION_ALIAS("c_readonly", ram)
REGION_ALIAS("c_writeable", low)
SECTIONS {
    /DISCARD/ : { *(.init.250) }
    .basic_header : { *(.basic_header) } >ram
    /* Keep growing ZP initializers out of the page-aligned raster prefix. */
    .zp.data : { INCLUDE zp-data-sections.ld } >zp AT>initializers
    INCLUDE zp-data-symbols.ld
    INCLUDE zp-bss.ld
    INCLUDE zp-noinit.ld
    /* Only startup validation/calibration uses generic wide arithmetic. Playback uses bounded
     * quotients, so these helpers can retire with the startup overlay. */
    .bootmath : {
        *(.text.__udivsi3 .text.__udivmodsi4 .text.__umodsi3)
        *(.text.__mulsi3)
        /* CRT memory initialization is complete before main. These helpers
         * share the startup overlay and are never needed by playback. */
        *(.text.__copy_data .text.__copy_zp_data .text.__zero_bss .text.__zero_zp_bss)
    } >bootwork
    /* Fill the otherwise unused gap before the page-aligned raster kernel. */
    .earlycode : {
        *(.earlycode .earlycode.*)
        /* The tiny disk dispatch table fits the remaining alignment gap. */
        *(.rodata.disk_step)
    } >ram
    .text : {
        _init = .; _start = .;
        KEEP(*(SORT_BY_INIT_PRIORITY(.init.* .init)))
        KEEP(*(.call_main)) KEEP(*(.after_main))
        _fini = .;
        KEEP(*(SORT_BY_INIT_PRIORITY(.fini.* .fini))) KEEP(*(.fini_rts))
        /* No fall-through past fini_rts: use the raster alignment gap. */
        *(.text.uci_start)
        /* Tiny leaf routines fill the padding before each aligned kernel. One
         * that outgrows its gap only pushes the kernel to its next boundary. */
        *(.text.havoc_tilt)
        *(.text.exit) *(.text.reu_execute) *(.text.__ashlqi3)
        *(.text.havoc_audio)
        *(.text.disk_pin) *(.text.disk_release) *(.text._ZN*9musicdisk5start*) *(.text.__lshrqi3)
        *(.text.pano_copy_band)
        *(.text.pano_picture)
        *(.text.pano_copy_row)
        *(.text .text.* CODE LOWCODE ONCE STARTUP)
        __init_array_start = .;
        KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.* .init_array)))
        __init_array_end = .;
        __fini_array_start = .;
        KEEP(*(SORT_BY_INIT_PRIORITY(.fini_array.* .fini_array)))
        __fini_array_end = .;
    } >ram
    INCLUDE rodata.ld
    .data : {
        __data_start = .;
        *(.lowcode .lowcode.*)
        *(.data .data.* DATA)
        __data_end = .;
    } >low AT>initializers
    INCLUDE data-symbols.ld
    INCLUDE bss.ld
    INCLUDE noinit.ld
    .highcode : {
        __high_start = .;
        *(.highcode .highcode.*)
        __high_end = .;
    } >high AT>initializers
    __high_load = LOADADDR(.highcode);
    .midcode : {
        __mid_start = .; *(.midcode .midcode.*) __mid_end = .;
    } >mid AT>initializers
    __mid_load = LOADADDR(.midcode);
    .tailcode : {
        __tail_start = .; *(.tailcode .tailcode.*) __tail_end = .;
    } >low AT>initializers
    __tail_load = LOADADDR(.tailcode);
    .bankcode : {
        __bank_start = .; *(.bankcode .bankcode.*) __bank_end = .;
    } >bankram AT>initializers
    __bank_load = LOADADDR(.bankcode);
    .disk_init : { *(.disk_init .disk_init.*) } >bootwork
}
/* asm/startup.S owns banking and interrupt shutdown before any CRT writes. */
__stack = 0xD000;
ASSERT(__heap_start <= 0x0800, "resident data exceeds low RAM")
ASSERT(__high_end <= 0xFFFA, "resident high code overlaps interrupt vectors")
ASSERT(__mid_end <= 0xCF80, "resident mid code overlaps 128-byte software stack")
ASSERT(__tail_end <= 0x0800, "resident tail code overlaps BASIC header")
ASSERT(__bank_end <= 0xE000, "pure calculation code exceeds RAM under I/O")
/* CRT copies these initializers before raster_init overwrites the cache. */
OUTPUT_FORMAT { SHORT(ORIGIN(ram)) FULL(ram) FULL(initializers) TRIM(bootwork) }
