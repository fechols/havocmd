"""Run the offline production suite in Linux/WSL. Never contacts hardware."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from build_fingerprint import (COMPONENT_REPORTS, digest, package_inputs, require,
                               test_sources, verify_build, verify_rust_tests, verify_rust_layout)

ROOT = Path(__file__).resolve().parents[1]


def main():
    os.chdir(ROOT)
    verify_build()
    source_before, package_before = test_sources(), package_inputs()
    results = []
    with tempfile.TemporaryDirectory(prefix='havoc-native-') as temporary:
        for name in ('panorama', 'disk-loader', 'camera-edges'):
            executable = Path(temporary) / name
            subprocess.run([os.environ.get('CC', 'cc'), '-std=c99', '-O1', '-g',
                            '-Wall', '-Wextra', '-Werror', '-Wno-misleading-indentation',
                            '-Wno-sign-compare', '-fsanitize=address,undefined',
                            '-ffunction-sections', '-fdata-sections',
                            str(ROOT / f'tools/test-{name}.c'), '-Wl,--gc-sections',
                            '-o', str(executable)], check=True)
            result = subprocess.run([str(executable)], check=True, text=True, capture_output=True)
            print(result.stdout, end='', flush=True)
            results.append({'test': name, 'passed': True, 'output': result.stdout.strip()})
    for name in ('package-gates', 'rust-layout', 'rust-player', 'audio-player', 'audio-fast', 'memory', 'bank', 'pano-copy', 'tilt', 'timer', 'musicdisk-startup'):
        command = [sys.executable, '-B', str(ROOT / f'tools/test-{name}.py')]
        if name == 'musicdisk-startup':
            command.extend(['--frames', '600'])
        subprocess.run(command, check=True)
        results.append({'test': name, 'passed': True})
    verify_build()
    verify_rust_tests()
    verify_rust_layout()
    require(source_before == test_sources(), 'source or configuration changed while tests were running')
    require(package_before == package_inputs(), 'build or content changed while tests were running')
    report = {'application_language': 'Rust', 'offline_tests_passed': True, 'hardware_measured': False, 'tests': results,
              'prg_sha256': digest(ROOT / 'dist/havocmd/havocmd.prg'),
              'source_sha256': source_before, 'package_sha256': package_before,
              'component_report_sha256': {name: digest(ROOT / path)
                                         for name, path in COMPONENT_REPORTS.items()}}
    (ROOT / 'dist/havocmd/offline-tests.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    print('All offline production checks passed; physical C64U timing is unmeasured.')


if __name__ == '__main__':
    main()
