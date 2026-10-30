# HAVOC musicdisk encoder

This standalone Rust tool ports the existing H.A.V.O.C. DLT5 encoder from
`sueme64/tools/encode-digital-music`. The underlying Second Reality tools carry
the included `UNLICENSE`; H.A.V.O.C. additions retain `HAVOC-LICENSE`. The
recordings have their separate license in `assets/music/LICENSE`.

The musicdisk profile is **`stereo32k3`**: true left/right stereo at 32,000 frames
per second, three-bit delta codes per channel, signed-16 saturating predictors,
and rounded signed-8 DAC output. It performs no mono downmix or artificial
channel delay. The comparison profile `stereo24k3` uses the same stereo DLT5
format at 24,000 frames per second. Existing `stereo20k3`, `mono16k3`, and legacy
DLT3 remain available for regression checks.

```powershell
cargo test --manifest-path tools/encode-digital-music/Cargo.toml --target-dir target/music/encoder
cargo build --release --manifest-path tools/encode-digital-music/Cargo.toml --target-dir target/music/encoder
.\target\music\encoder\release\encode-digital-music.exe --profile stereo32k3 --codebooks 96 --training combined --beam-width 128 input.flac output.dlt preview.wav
```

FFmpeg and FFprobe must be on PATH. Input FLAC retains the recording's original
rate and stereo channels; the encoder owns resampling, shared gain, and
quantization. `tools/import_music.ps1` authenticates the original recording and
prepares that FLAC. Its default is a short Hammond Rocker diagnostic; full
catalog encoding requires the hardware timing gate report.

The encoder writes `output.dlt`, adjacent `output.json`, a decoded preview, and
`preview.reference.wav`. Preview samples are the actual signed-8 output expanded
by 256 into signed-16 WAV containers. Metadata includes sample counts, byte
extents, checksums, hashes, shared gain, and per-channel error/peak measurements.
The original source attribution and hashes are retained by the import wrapper.

## DLT5 interface

The wire format is unchanged. All multibyte fields are little endian:

| Header offset | Field |
| --- | --- |
| 0 | `DLT5` magic |
| 4 | u32 sample rate (32000, or 24000 for the comparison profile) |
| 8 | u32 stereo frame count |
| 12 | u32 packed stream bytes |
| 16 | u16 codebook entries per channel (8) |
| 18 | u16 codebook count |
| 20 | bits per channel sample (3) |
| 21 | channels (2) |
| 22–31 | zero |

Every codebook is 32 bytes: eight signed-16 deltas for the left channel, then
eight for the right. Divide `ceil(frames/4)` complete packing groups evenly
among books, assigning one extra group to each of the first remainder books.
Clip only the final group. Predictors continue across book changes.

The continuous LSB-first stream alternates left and right three-bit codes.
Four stereo frames occupy three bytes. Final unused bits are zero. After the
stream is one cumulative CRC-8 byte per 1024 stream bytes (polynomial `$07`,
initial zero, no reset between chunks). Asset size is
`32 + 32*books + ceil(6*frames/8) + ceil(stream_bytes/1024)`.

For each code, saturating-add its delta to that channel's signed-16 predictor.
Publish `clamp(floor((predictor+128)/256), -128, 127)` as a signed-8 byte; do not
replace the predictor with the rounded result. Reset predictors at song start.

The existing independent bit oracle, balanced-segment cases, saturation and
all-65536-input rounding checks cover the new profile. A passing host suite
does not establish simultaneous audio/video throughput on C64 Ultimate.
