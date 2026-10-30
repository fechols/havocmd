# C64U NTSC HDMI raster gate

The first prototype establishes the usable display area and the audio/video
timing budget before the full musicdisk is packaged. Use a C64 Ultimate with
NTSC selected and software-controlled 64 MHz turbo enabled. Record the firmware
version, HDMI output mode, turbo setting, display model and overscan setting.
Do not use an emulator screenshot as proof of the visible outer edges.

1. Launch the prototype and use F5 to select the moving diagnostic grid.
   Check horizontal, vertical and diagonal motion. Narrow columns
   alternate colors so an incorrectly sized pixel or swapped packed nibble is
   conspicuous; horizontal grid boundaries expose row repetition and omission.
2. Adjust vertical start and horizontal delay using the prototype controls.
   Find the earliest/latest source edges visible in this HDMI mode. A correct
   calibration fills the C64 picture area including its usual border. The HDMI
   scaler's pillarboxing outside that picture is not addressable by `$D020`.
3. Observe the leftmost/rightmost colored columns and topmost/bottommost rows
   while the camera crosses both cache wraps. There must be no fixed C64 border
   band, one-column gap, missing final pixel or recurring row splice.
4. The fixed-time stereo work hook and blank-time producer run automatically.
   Check both channels separately and together. Record underrun/late-line counters
   before and after at least 60 seconds, including diagonal scrolling. A stable
   picture by itself does not establish sufficient audio throughput.
5. Read the startup REU bandwidth measurements using F1, then use F3 to enable
   the UCI stress path. Require the same edge coverage and continuous playback. Loader stress must
   use the disjoint `$400000` destination while audio and graphics continue.
   No active song, picture or PCM ring is overwritten. No loading screen is used.

Faint horizontal softness is acceptable. A lost edge, a repeated/missing frame,
an audio underrun or a systematic line deadline miss is a gate failure. Keep
the rendering area fixed while resolving a failed schedule; do not silently
shrink it to make audio fit.

Record this compact result for each configuration:

```text
Firmware / C64U model:
NTSC / HDMI output mode / display overscan:
Prototype build identifier:
Top line / horizontal delay:
All four C64 picture edges covered: yes / no
Even/odd horizontal phase and vertical cache wrap: pass / fail
Stereo left / right / simultaneous: pass / fail
Test duration and counter deltas:
REU stress / UCI stress / combined: pass / fail / not run
Observed defects and capture filename:
```

Only report hardware verification once these observations exist. The supplied
instruction trace proves the generated program's CPU schedule under its stated
model; it cannot prove FPGA bus arbitration, HDMI cropping or analog timing.
