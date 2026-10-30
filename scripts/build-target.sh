#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
sdk="${LLVM_MOS:-$HOME/.local/llvm-mos}"
mkdir -p target-c64/build dist/prototype
python3 target-c64/tools/generate_raster.py
"$sdk/bin/mos-c64-clang" -Os -g -fno-static-stack -Itarget-c64/include \
  -T target-c64/link.ld -Wl,-Map,target-c64/build/havocmd.map \
  target-c64/prototype_main.c target-c64/platform.c \
  target-c64/asm/startup.S target-c64/asm/raster.S target-c64/asm/audio_step.S \
  -o dist/prototype/havocmd-prototype.prg
"$sdk/bin/llvm-nm" -n dist/prototype/havocmd-prototype.prg.elf > target-c64/build/symbols.txt
echo 'Built dist/prototype/havocmd-prototype.prg (hardware timing acceptance pending)'
