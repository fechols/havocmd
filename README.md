# HAVOC Musicdisk for C64 Ultimate

This is a **production candidate; physical C64U validation is still pending**.
The production profile contains all 21 complete recordings at 16 kHz true stereo,
with signed 8-bit playback, and a full scrolling panorama for every song.
The player targets a 200 × 240 logical image with double-width pixels and all
16 colors, drawn through the C64 border-color register across the usual borders.

The soundtrack lasts 34:37.8835 before returning to Hammond Rocker. Songs retain
their complete source duration, including natural silences and endings.
Compressed DLT5 data is decoded in real time; the bundle contains no predecoded
soundtrack. The player prefetches the next song and picture while the current
pair remains active, then joins their PCM streams without adding a loading gap.
Published audio stays about a quarter second ahead of the sampler. The outgoing
song fades out over its final second; a manual skip ends the current song one
second after the key press, so the next one begins as the fade reaches silence.
Panoramas stay level and scroll smoothly in two dimensions. The automatic camera
reverses at the artwork edges, and each song change slides in its matching picture.
The camera keeps scrolling through the fade; it pauses only for the final quarter
second, while the next picture's seed loads, and the slide begins with the new song.

## Run

Copy the complete contents of `dist/havocmd` into a folder named `havocmd`
at the root of the SD card or USB drive. Keep these files together:

- `havocmd.prg`
- `havocmd-1.reu` and `havocmd-2.reu` (the packer writes as many 16 MiB volumes
  as the content needs, up to four; volume 1 records the count)
- `havocmd.cfg`, `README.txt`, and the supplied manifests and licenses

Finalizing also writes `dist/havocmd-musicdisk.zip`, which extracts to that
`havocmd/` folder.

Select NTSC and HDMI output, apply `havocmd.cfg`, and run `havocmd.prg`.
**No REU image needs to be preloaded or mounted manually.** The 16 MiB `.reu`
files are storage volumes. The program uses the Ultimate Command Interface to
find its current folder, configured Home, or `/havocmd/` on an actual mounted
device. It reads the required ranges into the machine's single 16 MiB REU.

A black screen during startup is expected while the first pair loads, the
sampler is calibrated, and at least 3,968 stereo frames are prepared. Picture and
playback then start together. The configuration enables 64 MHz software
controlled turbo, disables badline timing, enables REU/UCI/Ultimate Audio,
disables REU preload, and leaves automatic configuration saving off.

See [the supplied instructions](target-c64/docs/musicdisk-readme.txt) for
controls and startup failure colors. The PRG and all volumes must come
from the same build: the program checks their directory against its compiled
checksum.

## Controls

- **F1 / F3:** previous / next song; the current pair stays active while its
  replacement is prepared.
- **Space:** next song, once per press.
- **F5:** resume automatic panning.
- **F7:** mute or unmute; playback continues.
- **Joystick port 2:** pan manually. **Fire:** resume automatic panning.
- Reset the C64U to exit.

The automatic playlist follows `assets/music/catalog.json`, starts with
Hammond Rocker, plays every full song, and wraps after Galactic Dash.
The Shattered Moon picture is paired with the soundtrack's Lunar Awe.

## Content and validation

All 21 source images are 1448 × 1086. They are halved with a Lanczos3 filter and
aspect-corrected in the same pass before quantization, becoming 453 × 543
logical panoramas, padded to 464 × 544 for storage. Each includes a
32 KiB starting cache followed by native 16 × 8 tiles. Scrolling covers the full
picture rather than only its starting cache.

The packed assets use 28,138,321 bytes of compressed music and 3,338,496 bytes
of artwork. Including alignment, 31,479,552 of the two volumes' 33,546,240
payload bytes are occupied; 2,066,688 bytes remain free, and each further
volume would add 16,773,120. Packaging checks these
sizes against the completed encodings. The current and next
song/picture bundles fit in separate 5 MiB REU slots.

The 16 kHz profile reduces decoder and transfer work so more of each frame is
available for scrolling and background loading. Offline checks exercise the
linked Rust PRG for at least 600 consecutive frames. The camera must travel at
least 285 pixels on each axis over 600 frames, without either axis remaining stationary for
more than four consecutive frames. The model compares every produced stereo
byte with an independent decoder, checks every rendered pixel against the
original artwork, and verifies raster cadence, loader progress and startup
relocation. The camera remains level throughout.
Continuous playback and timing headroom still need physical hardware validation.

Packaging verifies original-source provenance, complete frame counts, encoding
settings, DLT5 stream checksums, every image seed against its native tiles, all
volume headers, and every asset across volume boundaries. SHA-256 hashes are in
`musicdisk-manifest.json`; recording provenance is in `music-provenance.json`.
These content checks and host CPU/peripheral models do not establish physical
audio/video deadlines or HDMI edge coverage. Reports keep hardware verification
false until the production candidate is measured on the C64U.

The older `dist/prototype` and `target/experiments24` artifacts are historical
diagnostics. Their controls, preload instructions and reports do not validate
the current production sources or full musicdisk.

## Build

The C64 application is a `no_std` Rust binary in `target-c64/src`. Its raster,
pixel loop, sample decoder, cache copy kernels, and timer helpers use 6502 assembly. The
earlier C implementation is retained as an independent behavioral reference;
it is not compiled into the production PRG.

Use host Rust with the locked dependencies, FFmpeg and FFprobe on PATH, Python 3,
and LLVM-MOS in WSL at `$HOME/.local/llvm-mos` or the `LLVM_MOS` location.
The C64 build uses the installed `mos` Rust toolchain and the `mos-c64-none`
target, following the neighboring `sueme64` project. Set `RUST_TARGET_PATH` to
the directory containing that target's JSON definition (the default is
`$HOME/.local/rust-mos-targets`). Cargo rebuilds `core` for this target and links
through LLVM-MOS with whole-program optimization and a 110-byte zero-page pool.
IRQs stay disabled during playback. Pure picture setup and playlist lookup use RAM
under I/O through short assembly gates that restore the normal memory map before
rendering or device access. The linked-code audit checks these gates and their
entire direct call graph; the boot model keeps that RAM separate from I/O.
The build records both compiler identities
and source hashes so stale binaries cannot pass the packaging gate.
Original music files are tracked with Git LFS; run `git lfs pull` after cloning.

From the repository root in PowerShell:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/build-musicdisk.ps1
```

The script uses the `Ubuntu-24.04` WSL distribution by default (override with
`-Distribution`). It resumes authenticated encodings, bakes the art, verifies
and packs all content, compiles the PRG, runs the offline production suite, and
generates the runtime manifest and `SHA256SUMS.txt`. `-SkipEncoding` uses existing
full 16 kHz encodings in `target/music/full16`; packaging still validates their
provenance and contents and rejects another audio profile.

`-StageOnly` permits offline encoding and records hardware verification as
false. `-Resume` authenticates existing outputs before reusing them. The importer
preserves the original stereo channels and full decoded duration; the encoder
uses three-bit delta codes per channel with signed 16-bit predictors and rounds
to signed 8-bit output.

Package the content before building the PRG: packaging generates
`target-c64/include/musicdisk_assets.h` with the exact directory checksum.
The volume format is version 2. Regenerate the matching PRG after changing any
asset or directory metadata. `cargo run --offline --release -- check dist/havocmd`
checks the storage files independently.

`scripts/build-c-reference.sh` builds the C comparison PRG separately under
`target/c-reference`. Four native Rust models exercise audio, panorama streaming,
loading and controls with overflow checks enabled. The instruction model boots the actual linked PRG from dirty RAM and checks
stereo PCM, raster cadence, loader progress, and memory ownership.

After building, copy only the verified runtime bundle with Python:

```powershell
python tools/finalize-musicdisk.py --destination D:\havocmd
```

This copies and hashes every runtime file, without deleting other files on the
destination. The ELF and linker map remain local for debugging.

## Attribution

Project code is MIT licensed. The recordings and artwork belong to William
Echols; the music license is `assets/music/LICENSE`. The vendored encoder
retains `tools/encode-digital-music/UNLICENSE` and `HAVOC-LICENSE`.
The reused CPU interpreter retains `tools/native_cpu.LICENSE`.
