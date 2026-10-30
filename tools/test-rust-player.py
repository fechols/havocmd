"""Run all production Rust host models against an authenticated full16 song.

Uses a host compiler, never the MOS target or C64U hardware. The instruction
models separately verify the assembled kernels and complete linked PRG.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from build_fingerprint import (RUST_COMPONENTS, RUST_REPORT_FORMAT, digest,
                               require, rust_test_sources, snapshot)

ROOT = Path(__file__).resolve().parents[1]


def full_song():
    spec = importlib.util.spec_from_file_location('rust_player_content_validator',
                                                ROOT / 'tools/prepare-musicdisk.py')
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    catalog = json.loads((ROOT / 'assets/music/catalog.json').read_text(encoding='utf-8-sig'))
    require(catalog['profile'] == 'stereo16k3' and catalog['sample_rate'] == 16000,
            'Rust audio tests require the production stereo16k3 catalog')
    track = next(t for t in catalog['tracks'] if t['name'] == 'hammond-rocker')
    require(track['id'] == 1 and track['expected_frames'] > 80000,
            'Rust audio tests require the complete Hammond song, not a diagnostic excerpt')
    folder = ROOT / 'target/music/full16'
    files = [folder / track['asset'], folder / track['metadata'],
             folder / 'hammond-rocker.provenance.json', folder / 'hammond-rocker-source.flac']
    require(all(path.is_file() for path in files),
            'missing authenticated full16 Hammond fixture; run the full-catalog importer first')
    asset = validator.validate_track(track, folder)
    require(asset['sha256'] == track.get('asset_sha256'), 'full16 fixture differs from the catalog hash')
    return asset, files


def host_compiler():
    explicit = os.environ.get('HAVOC_HOST_RUSTC')
    if explicit:
        executable = Path(explicit).resolve()
        require(executable.is_file(), 'HAVOC_HOST_RUSTC must name the actual host compiler')
        return [str(executable)], executable
    rustup = shutil.which('rustup')
    if not rustup:
        candidate = Path.home() / '.cargo/bin' / ('rustup.exe' if os.name == 'nt' else 'rustup')
        if candidate.is_file():
            rustup = str(candidate)
    require(rustup, 'install the stable host Rust toolchain or set HAVOC_HOST_RUSTC')
    executable = Path(subprocess.check_output([rustup, 'which', '--toolchain', 'stable', 'rustc'],
                                             text=True).strip())
    return [rustup, 'run', 'stable', 'rustc'], executable


def main():
    destination = ROOT / 'target/rust-player-tests.json'
    destination.unlink(missing_ok=True)  # Failed reruns cannot leave an old success report.
    source_before = rust_test_sources()
    asset, fixture_files = full_song()
    fixture_before = snapshot(fixture_files)
    compiler, executable = host_compiler()
    version = subprocess.check_output([*compiler, '-vV'], cwd=ROOT, text=True).strip()
    cases = []
    with tempfile.TemporaryDirectory(prefix='havoc-rust-player-') as temporary:
        for component in RUST_COMPONENTS:
            source = ROOT / f'tools/test-{component}-rust.rs'
            output = Path(temporary) / (component + ('.exe' if os.name == 'nt' else ''))
            command = [*compiler, '--edition=2021', '--test', '-C', 'opt-level=2',
                       '-C', 'overflow-checks=yes', str(source), '-o', str(output)]
            subprocess.run(command, cwd=ROOT, check=True)
            environment = dict(os.environ, HAVOC_AUDIO_TEST_DLT=str(asset['path']))
            begin = time.monotonic()
            result = subprocess.run([str(output), '--test-threads=1', '--nocapture'],
                                    cwd=ROOT, env=environment, text=True, capture_output=True)
            print(result.stdout, end='', flush=True)
            if result.stderr:
                print(result.stderr, end='', flush=True)
            result.check_returncode()
            match = re.search(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; '
                              r'(\d+) measured; (\d+) filtered out;', result.stdout)
            require(match is not None, f'{component}: missing Rust test summary')
            passed, failed, ignored, measured, filtered = map(int, match.groups())
            require(passed > 0 and failed == ignored == measured == filtered == 0,
                    f'{component}: all Rust tests must run and pass')
            if component == 'audio':
                require('160000 output PCM bytes matched' in result.stdout,
                        'audio model did not exercise the required full-song fixture')
            cases.append({'component': component, 'passed': True, 'tests_passed': passed,
                          'tests_failed': failed, 'tests_ignored': ignored, 'tests_filtered': filtered,
                          'elapsed_seconds': round(time.monotonic() - begin, 3),
                          'output': result.stdout.strip()})
    require(source_before == rust_test_sources(), 'Rust model sources changed while tests were running')
    require(fixture_before == snapshot(fixture_files), 'full-song fixture changed while tests were running')
    report = {'format': RUST_REPORT_FORMAT, 'application_language': 'Rust', 'passed': True,
              'hardware_measured': False, 'overflow_checks': True, 'test_threads': 1,
              'compiler_version': version, 'compiler_sha256': digest(executable),
              'source_sha256': source_before, 'fixture_sha256': fixture_before,
              'fixture': {'id': 1, 'name': 'hammond-rocker', 'profile': 'stereo16k3',
                          'sample_rate': 16000, 'frames': asset['frames'], 'bytes': asset['bytes'],
                          'sha256': asset['sha256'], 'decoded_pcm_bytes_checked': 160000},
              'cases': cases}
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print('All four production Rust models passed with overflow checks and the authenticated full16 song.')


if __name__ == '__main__':
    main()
