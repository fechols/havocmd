#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/env.sh
mkdir -p target-c64/build dist/havocmd
python3 tools/generate_tilt.py
python3 tools/build_fingerprint.py begin --compiler "$MOS_CLANG" --rust-compiler "$(rustup which --toolchain mos rustc)" --rust-target "$RUST_TARGET_PATH/mos-c64-none.json"
(cd target-c64 && cargo build --offline --locked --release)
native_prg=target-c64/target/mos-c64-none/release/havocmd-c64
elf=
for candidate in target-c64/target/mos-c64-none/release/deps/havocmd_c64-*.elf; do
    if [[ "${candidate%.elf}" -ef "$native_prg" ]]; then
        [[ -z "$elf" ]] || { echo 'Ambiguous Cargo ELF identity' >&2; exit 1; }
        elf=$candidate
    fi
done
[[ -n "$elf" ]] || { echo 'Missing matching Cargo ELF' >&2; exit 1; }
cp "$native_prg" dist/havocmd/havocmd.prg
cp "$elf" dist/havocmd/havocmd.prg.elf
"$LLVM_MOS/bin/llvm-nm" -n dist/havocmd/havocmd.prg.elf > target-c64/build/musicdisk-symbols.txt
python3 tools/build_fingerprint.py finish
echo 'Built Rust/assembly 16 kHz stereo dist/havocmd/havocmd.prg; hardware validation pending.'
