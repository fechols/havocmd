#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
sdk="${LLVM_MOS:-$HOME/.local/llvm-mos}"
mkdir -p target/c-reference
python3 tools/generate_tilt.py
"$sdk/bin/mos-c64-clang" -Oz -g -DHAVOC_PRODUCTION=1 -DAUDIO_SAMPLE_RATE=16000UL -Itarget-c64/include \
  -T target-c64/link.ld -Wl,--emit-relocs -Wl,-Map,target/c-reference/musicdisk.map \
  target-c64/main.c target-c64/audio.c target-c64/platform.c target-c64/musicdisk.c \
  target-c64/uci.c target-c64/panorama.c \
  target-c64/asm/startup.S target-c64/asm/tilt.S target-c64/asm/audio_step.S target-c64/asm/audio_fast.S target-c64/asm/timer.S \
  -o target/c-reference/havocmd.prg
"$sdk/bin/llvm-nm" -n target/c-reference/havocmd.prg.elf > target/c-reference/musicdisk-symbols.txt
echo 'Built isolated C reference target/c-reference/havocmd.prg.'
