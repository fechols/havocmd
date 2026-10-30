HAVOC C64U MUSICDISK - HARDWARE PROTOTYPE R3 (RASTER CALIBRATION)

This is the timing test, not the complete 21-song musicdisk.
It contains a two-second stereo Hammond Rocker loop and a moving crop of
Lowland Approach. Audio is decoded live from compressed DLT5 data to 8-bit PCM.
The picture targets 200 x 240 double-width pixels through D020, including the
usual C64 borders. Full HDMI edge coverage has not yet been measured.

R3 fixes the wavy/uneven pixel widths (horizontal delay13), the bottom black
strip (top31), and the eight-dot shift on raster zero. The cropped network
picture matched all pixels for126 consecutive frames after audio stopped.
The full32k stereo timing test still fails: playback may initially flicker
and then stop on underrun, leaving smooth graphics. This is a known scheduler
limitation, not a successful musicdisk build. Codebook reuse improves throughput
but does not yet eliminate the misses. Full24k/32k comparison is in progress.

START
Put this folder at /havocmd-prototype on USB. Select NTSC video on the C64U.
Apply havocmd-prototype.cfg and run havocmd-prototype.prg. The matching
havocmd-prototype.reu must be loaded into the 16 MB REU. If a relative preload
path is not accepted, select that image manually in the Ultimate file browser.
The config enables 64 MHz, disables badline timing, enables Ultimate Audio and
the Command Interface. Use HDMI for the edge test; disable TV overscan.
Startup briefly calibrates the silent sampler and primes the audio ring.

R2 fixes the missing low-RAM code/data copy that made the first build crash
before drawing or starting audio. It also disables interrupts before the CRT
overwrites low RAM. Tests now boot the actual delivered PRG from its BASIC SYS
entry, rather than starting only at individual routines in preinitialized RAM.

STARTUP COLORS
Blue: startup/relocation. Light blue: cache, REU and DMA initialization.
Yellow: silent sampler rate calibration. Green: audio priming starts.
Persistent red: matching REU image was not found in REU memory.
Persistent purple: Ultimate Audio sampler is not enabled/recognized.
Persistent orange: sampler calibration failed.
These failure colors work without the border graphics renderer. Reset after
correcting configuration and reload the matching REU before running again.

CONTROLS
Joystick port 2: move within the crop. Fire: automatic diagonal tour.
F1: toggle diagnostic numbers and freeze camera (audio continues).
F3: toggle background 16 KiB UCI file reads into disjoint REU scratch space.
F5: toggle calibration grid / Lowland Approach crop.
F7: mute/unmute, without pausing the decoder or playback clock.
+ / -: adjust horizontal timing; Shift with + / -: adjust top raster line.
Reset the C64U to exit. Cursor controls and large panorama streaming are not
part of this first diagnostic.

F1 COUNTERS (hexadecimal, left to right)
Top row:
  late scanlines | audio underruns | late frames | completed UCI reads |
  audio fault then UCI error (two digits each) | minimum queued audio frames |
  most recent audio-service time in CIA ticks
Second row:
  1536-byte DMA probe ticks | 2560-byte DMA probe ticks | top line |
  horizontal delay | measured sampler divider
At NTSC, one CIA tick is about 0.978 microseconds. The first two values on the
second row approximate 8-bit and 16-bit audio traffic per display frame.
They measure transfer cost only; this PRG does not yet play 16-bit PCM.
Diagnostic drawing and toggling the grid/art add work, so compare counter
deltas during an uninterrupted run, then use F1 to read them.

Audio faults: 01 missing/wrong REU header, 02 sampler not found, 03 sampler
calibration failed, 04 invalid compressed chunk, 05 underrun, 06 long service
stall. Playback stops on a runtime fault so a failure is not hidden.
UCI errors: 01 interface absent, 02 timeout, 03 interface error, 04 file-command
failure, 05 short/unrecognized read count. Audio remains independent of UCI.

HARDWARE TEST
Find top/delay settings covering all four C64 picture edges. HDMI pillarboxes
outside the C64's generated picture cannot be painted with D020. Check both
horizontal phases and diagonal movement. Faint softness is acceptable; fixed
border strips, missing rows or edges are not.
Run at least 60 seconds with stereo playback, then with F3 reads enabled.
Record firmware, HDMI mode, top/delay, both counter rows and visible defects.
Increasing late-line/frame or underrun counters fails the timing gate. A
repeating two-second clip is intentional; silence or stuttering is not.

The software tests pass the modeled CPU schedule and decoded sample checks.
Only this physical test can establish FPGA arbitration and visible coverage.
The full playlist, four storage volumes and background panorama streamer
follow that test. No physical pass is claimed in the JSON build reports.
