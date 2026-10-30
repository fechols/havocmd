"""Production C producer + current linked 6502 decoder tests, without hardware.

Run from Linux/WSL: python3 tools/test-audio-player.py
The C model verifies complete PCM sequences and scheduler ownership. The
separate NMOS6502 oracle compiles today's assembly, verifies its exact outputs,
and checks every callback path still takes256 nominal CPU cycles.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cc', default=os.environ.get('CC', 'cc'))
    parser.add_argument('--mos-clang', default=os.environ.get(
        'MOS_CLANG', str(Path.home()/'.local/llvm-mos/bin/mos-c64-clang')))
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('prototype_tests', ROOT/'tools/test-prototype.py')
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    with tempfile.TemporaryDirectory(prefix='havoc-audio-') as directory:
        directory = Path(directory)
        native = directory/'native-player'
        subprocess.run([args.cc, '-std=c99', '-O1', '-g', '-Wall', '-Wextra', '-Werror',
                        '-fsanitize=address,undefined', str(ROOT/'tools/test-audio-player.c'),
                        '-o', str(native)], check=True)
        subprocess.run([str(native)], check=True)
        selection = directory/'native-selection'
        subprocess.run([args.cc, '-std=c99', '-O1', '-g', '-Wall', '-Wextra', '-Werror',
                        '-Wno-misleading-indentation', '-Wno-sign-compare',
                        '-ffunction-sections', '-fdata-sections',
                        '-fsanitize=address,undefined', str(ROOT/'tools/test-player-selection.c'),
                        '-Wl,--gc-sections', '-o', str(selection)], check=True)
        subprocess.run([str(selection)], check=True)
        fixture = directory/'kernel-fixture.c'
        fixture.write_text('''#include "audio_step.h"
volatile uint8_t havoc_audio_chunk_state[6],havoc_audio_chunk_units[6],havoc_audio_chunk_reset[6];
uint8_t havoc_audio_codes_hi[6],havoc_audio_pcm_hi[6],havoc_audio_lut_hi[6];
int main(void) { havoc_raster_work();return 0; }
''')
        prg = directory/'kernel-fixture.prg'
        subprocess.run([args.mos_clang, '-Oz', '-I'+str(ROOT/'target-c64/include'), str(fixture),
                        str(ROOT/'target-c64/asm/audio_step.S'), '-o', str(prg)], check=True)
        memory, symbols = oracle.image(Path(str(prg)+'.elf'))
        samples = oracle.test_audio(memory, symbols)
        report = {
            'production_c_producer_passed': True,
            'sample_rate_hz': 16000,
            'production_selection_passed': True,
            'asan_ubsan_passed': True,
            'assembly_pcm_samples_checked': samples,
            'callback_nominal_cpu_cycles': 256,
            'tests': ['exact PCM across distinct tracks with shared book numbers',
                      'predictor reset on track, manual cut, and retry loop boundaries',
                      'complete compressed sources and ring wraps',
                      'audible boundary events and pending-boundary requeue guard',
                      'counter rebase with uneven codebook segment lengths',
                      'pending audible boundary survives counter rebase',
                      '4227 bounded-division cases across the16k ring-duration domain including maximum clock interval and remainder',
                      'full ring retention and deadline guard', 'clock gap and underrun stop'],
            'hardware_measured': False,
            'source_sha256': {name: hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in
                              ['target-c64/audio.c', 'target-c64/asm/audio_step.S',
                               'target-c64/main.c', 'tools/test-audio-player.c',
                               'tools/test-player-selection.c']},
        }
        destination = ROOT/'target-c64/build/audio-player-tests.json'
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(report, indent=2)+'\n')
        print('Audio player model and linked assembly oracle passed; hardware timing remains unmeasured.')


if __name__ == '__main__':
    main()
