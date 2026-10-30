# NTSC raster prototype contract

This is a hardware calibration prototype. The raster has been checked on a
C64U; the combined audio/video timing gate has not passed.
It paints a 200-by-240 logical viewport with two-dot-wide pixels through `$D020`.
The VIC display and sprites stay disabled; image coverage includes the border.
R3 uses starting raster line31 and horizontal delay13, measured on firmware
1.1.0, FPGA122/core1.49, NTSC at64MHz. Top-line adjustment is0..63; delay is1..24.
Capture the actual HDMI output to establish the correct boundaries: the network
video stream crops outer columns and cannot establish complete edge coverage.

## Memory and frame interface

The linker must reserve `$4000-$BFFF` for 128 page-aligned packed-byte columns.
Column c, row y is `$4000 + 256*c + y`; low nibble is x=2c, high nibble x=2c+1.
The 256-by-256 cache wraps in both directions. `havoc_raster_x/y` select a
200-by-240 view; the even and odd x phases use different straight-line kernels.
No image copy or nibble repack is needed to scroll. Patch source operands once
before each frame, then keep all camera/cache mutations outside active drawing.

`havoc_raster_init()` fills a diagnostic pattern and builds a 256-byte aligned
high-nibble lookup. Code and that table belong below `$4000`. `$C000-$CFFF` and
`$E000-$FFEF` are available to the main program subject to its own linker map.
ROMs stay banked out, IRQ sources disabled. `havoc_raster_frame()` returns just
after the 240th row so the caller can service audio, input, DMA and UCI in blank.
It waits for the next target raster line when called again; the caller must measure
its service deadline rather than infer a guaranteed frame rate from this API.

## Instruction schedule

`tools/generate_raster.py` emits `asm/raster_pixels.inc` and `raster-timing.json`.
All source loads are page-aligned absolute,Y reads; the nibble lookup is also
page aligned. Each group of eight logical pixels consumes 126 nominal CPU
cycles, matching 16 VIC dots **if** the hardware supplies 63 CPU cycles per
eight-dot PHI2 at the 64 MHz setting. Individual stores are spaced 14 or 16 CPU
cycles; the grouped cadence and sampling phase, not an ideal integer divider,
must be checked on hardware. Both x phases have the same total schedule.

At delay13, the line preamble is549 cycles:200 for25 immediate-operand
patches,273 for the work hook and Y preservation,72 for delay, and four for
the odd phase's first-byte fetch (matched by an absolute BIT in the even phase).
Pixel code is 3150 cycles. Its tail and RTS cost 14; with the outer JSR the
line call costs3719 cycles. These numbers exclude the `$D031` resynchronization,
frame loop, bus arbitration and instruction-fetch phase. A full NTSC scanline
is nominally 65 PHI2 cycles; an all-in timing trace must account for the slow
resynchronization accesses instead of treating them as ordinary turbo cycles.

On the measured NTSC core, raster zero's observed transition is one PHI2 late
relative to visible dots. Before that row, R3 subtracts12 delay iterations
(60CPU cycles) and replaces the three-NOP pad with a JMP past the same bytes
(another3cycles). It restores the regular pad and delay on the following row.
This exact63-cycle correction removes the observed eight-dot row shift. The
compensation is calibrated for the default delay13; very small manual delay
values clamp its loop count to1 and are not an accepted calibration.

The visible240-row picture spans31..262 and0..7. Requiring a fresh bottom/wrap
wait after drawing it would skip an entire refresh, so R3 polls the nine-bit
target line directly. The linked beam model tests both frame-wrap and ordinary
start positions, with both horizontal phases and source-row wrap.

## Physical results,2026-10-29

R2 delay12 produced periodic one/three-dot pixel widths. With delay13, both
camera parities match exact double-wide source pixels. Top31 removes the
17-row black band seen with top14. After the raster-zero correction,
`target/hardware/r3-raster-stable` contains126 consecutive384x240 frames with
zero pixel mismatches, zero wholly black frames and zero missing UDP packets.
This recording was taken after the audio underrun stopped its producer; it
validates the raster, not simultaneous sustained playback. The firmware's
cropped stream still cannot verify the outer16 HDMI columns.

The32k stereo producer exceeded frame deadlines and drained its initial ring.
Shared codebook/LUT reuse reduced modeled producer CPU cost by about37%, but
the physical test still encountered missed refreshes and fault5. The whole
musicdisk timing gate therefore remains failed. DMA probes measured2068CIA
ticks for1536bytes and3533ticks for2560bytes; treat those as measured bursts,
not a universal per-byte transfer rate. See saved captures and counters under
`target/hardware/` for the separate raster and combined-load observations.

## Bounded work hook

`havoc_raster_work()` executes before the pixels in every active line. A strong
assembly implementation is mandatory. It must take **exactly 256
nominal CPU cycles including RTS on every path**; the renderer separately pays
JSR and Y preservation. A/X/Y may be clobbered.
The renderer does not use compiler imaginary registers during drawing.

REU DMA or external I/O can add wall-time stalls; calling such instructions
does not satisfy the contract merely because their nominal instruction counts
sum to 256. Keep them outside this hook until a measured wall-time schedule
replaces the nominal contract. A variable-time C decoder is not an eligible
replacement. Integrate audio only after its fixed-time microkernel and staged
input/output buffers have been independently validated.

## Required checks

- Compile both phase kernels and verify actual disassembly against the emitted
  store schedule, including page crossings and the final pixel hold time.
- Compare rendered pixels against the cache reference for every x phase and
  for camera positions crossing x=255/y=255; confirm no 199/201-pixel row.
- Measure color transitions, first/last columns and all 240 rows on NTSC HDMI.
- Exercise delay/top calibration and monitor `havoc_raster_late_lines` (an
  eight-bit wrapping diagnostic, not a proof that no sub-line jitter occurred).
- Integrate the constant-time audio hook, then separately stress blank-time
  REU DMA/UCI service and check audio underruns and video coverage together.

Published hardware facts informing this independent implementation:
[Ultimate turbo](https://1541u-documentation.readthedocs.io/en/latest/config/turbo_mode.html),
[output and capture geometry](https://1541u-documentation.readthedocs.io/en/latest/data_streams.html),
[Upic timing description](https://github.com/xahmol/ultimate-uci-oscar64/blob/main/docs/UPIC_MANUAL.md).
No upstream renderer implementation is copied into this prototype.
