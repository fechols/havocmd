"""Finalize a tested bundle, optionally copy its exact manifest to removable media."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import zipfile
from build_fingerprint import (COMPONENT_REPORTS, digest, package_inputs, require,
                               test_sources, verify_build, verify_rust_tests, verify_rust_layout)

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE_FILES = ['README.txt', 'havocmd.cfg', 'havocmd.prg', 'LICENSE.txt', 'MUSIC-LICENSE.txt', 'SHA256SUMS.txt']


def write_archive(bundle, count):
    """Deterministic distribution zip: the player files under a havocmd/ folder."""
    archive = bundle.parent / 'havocmd-musicdisk.zip'
    names = ARCHIVE_FILES[:3] + [f'havocmd-{i}.reu' for i in range(1, count + 1)] + ARCHIVE_FILES[3:]
    with zipfile.ZipFile(archive, 'w') as output:
        for name in names:
            info = zipfile.ZipInfo(f'havocmd/{name}', date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            output.writestr(info, (bundle / name).read_bytes(), compresslevel=9)
    with zipfile.ZipFile(archive) as output:
        require(output.testzip() is None, 'archive failed its integrity check')
        for name in names:
            require(hashlib.sha256(output.read(f'havocmd/{name}')).hexdigest() == digest(bundle / name),
                    f'archive member differs from the bundle: {name}')
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, help='copy the verified runtime files into this folder')
    args = parser.parse_args()
    bundle = ROOT / 'dist/havocmd'
    content = json.loads((bundle / 'musicdisk-manifest.json').read_text())
    tests = json.loads((bundle / 'offline-tests.json').read_text())
    startup = json.loads((bundle / 'startup-tests.json').read_text())
    verify_build()
    require(tests.get('application_language') == 'Rust', 'offline suite must test the Rust application')
    verify_rust_tests(content)
    verify_rust_layout()
    prg_hash = digest(bundle / 'havocmd.prg')
    require(content['content_verified'] and tests['offline_tests_passed'] and startup['machine_code_boot_passed'],
            'content and offline tests must pass before packaging')
    require(prg_hash == tests['prg_sha256'] == startup['prg_sha256'], 'test reports do not match this PRG')
    require(tests['source_sha256'] == test_sources(), 'source, scripts, assets or configuration changed after testing')
    require(tests['package_sha256'] == package_inputs(), 'tested build or content metadata changed after testing')
    expected_tests = {'panorama', 'disk-loader', 'camera-edges', 'package-gates',
                      'rust-layout', 'rust-player', 'audio-player', 'audio-fast', 'memory', 'bank', 'pano-copy', 'tilt', 'timer', 'musicdisk-startup'}
    require({case['test'] for case in tests['tests'] if case['passed']} == expected_tests,
            'the complete offline suite must pass')
    cases = {case['case']: case for case in startup['cases']}
    require(set(cases) == {'production_start', 'missing_uci', 'bad_header', 'missing_sampler'} and
            all(case['passed'] for case in cases.values()) and
            cases['production_start'].get('continuous_frames', 0) >= 600,
            'startup report must cover all faults and at least600 continuous production frames')
    production = cases['production_start']
    camera = production.get('camera_scroll', {})
    require(production.get('rotation_enabled') is False and production.get('tilt_values') == [0] and
            camera.get('passed') is True and camera.get('input_injection') is False and
            camera.get('observed_frames') == production['continuous_frames'] and
            camera.get('distance_x',0) >= production['continuous_frames']*95//200 and
            camera.get('distance_y',0) >= production['continuous_frames']*95//200 and
            camera.get('maximum_consecutive_stationary_frames',999) <= 4 and
            camera.get('maximum_stationary_x_frames',999) <= 4 and
            camera.get('maximum_stationary_y_frames',999) <= 4 and
            0 < camera.get('maximum_frame_step',255) <= 1,
            'startup report must prove level scrolling with sustained camera movement and no long holds')
    require(production.get('first_picture_k') == 0 and
            production.get('all_frame_pixels_checked', 0) >= production['continuous_frames']*240*202 and
            production.get('original_art_pixels_checked', 0) >= production['continuous_frames']*240*200 and
            production.get('bank_bytes_checked', 0) > 0 and production.get('banked_instructions', 0) > 0 and
            production.get('raster_deadlines_passed') is True,
            'startup report must verify level first picture, original artwork, all pixels and raster deadlines')
    count = len(content['volumes'])
    require(1 <= count <= 4 and len(startup['volume_sha256']) == count,
            'content and startup reports must describe the same one to four volumes')
    for index, (volume, expected) in enumerate(zip(content['volumes'], startup['volume_sha256']), 1):
        name = f'havocmd-{index}.reu'
        require(volume['file'] == name and volume['bytes'] == (bundle / name).stat().st_size == 0x1000000 and
                volume['sha256'] == expected == digest(bundle / name),
                f'volume {index} does not match both validated content and startup tests')
    for name, path in COMPONENT_REPORTS.items():
        require(digest(ROOT / path) == tests['component_report_sha256'][name],
                f'component report changed after the suite passed: {name}')
    shutil.copyfile(ROOT / 'assets/musicdisk.cfg', bundle / 'havocmd.cfg')
    shutil.copyfile(ROOT / 'target-c64/docs/musicdisk-readme.txt', bundle / 'README.txt')
    shutil.copyfile(ROOT / 'target-c64/build/audio-player-tests.json', bundle / 'audio-player-tests.json')
    shutil.copyfile(ROOT / 'target/audio-fast-test/audio-fast-tests.json', bundle / 'audio-fast-tests.json')
    shutil.copyfile(ROOT / 'target/tilt-test/tilt-tests.json', bundle / 'tilt-tests.json')
    shutil.copyfile(ROOT / 'target-c64/build/timer-tests.json', bundle / 'timer-tests.json')
    shutil.copyfile(ROOT / 'target/rust-player-tests.json', bundle / 'rust-player-tests.json')
    shutil.copyfile(ROOT / 'target/memory-test/memory-tests.json', bundle / 'memory-tests.json')
    shutil.copyfile(ROOT / 'target/bank-test/bank-tests.json', bundle / 'bank-tests.json')
    shutil.copyfile(ROOT / 'target/pano-copy-test/pano-copy-tests.json', bundle / 'pano-copy-tests.json')
    shutil.copyfile(ROOT / 'target-c64/build/rust-layout-tests.json', bundle / 'rust-layout-tests.json')
    names = ['havocmd.prg', *[f'havocmd-{i}.reu' for i in range(1, count + 1)], 'havocmd.cfg', 'README.txt',
             'LICENSE.txt', 'MUSIC-LICENSE.txt', 'musicdisk-manifest.json', 'music-provenance.json',
             'build-inputs.json', 'offline-tests.json', 'startup-tests.json',
             'audio-player-tests.json', 'audio-fast-tests.json', 'tilt-tests.json', 'timer-tests.json',
             'rust-player-tests.json', 'memory-tests.json', 'bank-tests.json', 'pano-copy-tests.json', 'rust-layout-tests.json']
    report = {'format': 'havoc-runtime-package-v1', 'application_language': 'Rust', 'hardware_verified': False,
              'profile': content['profile'], 'full_songs': content['full_song_count'],
              'sample_rate': content['sample_rate'], 'channels': content['channels'],
              'output_bits': content['output_bits_per_sample'],
              'renderer': '200x240 logical border pixels, smooth two-dimensional scrolling, no rotation' ,
              'files': {name: {'bytes': (bundle / name).stat().st_size, 'sha256': digest(bundle / name)}
                        for name in names}}
    (bundle / 'build-report.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    names.append('build-report.json')
    sums = ''.join(f'{digest(bundle / name)}  {name}\n' for name in names)
    (bundle / 'SHA256SUMS.txt').write_text(sums, encoding='ascii')
    names.append('SHA256SUMS.txt')
    archive = write_archive(bundle, count)
    if args.destination:
        destination = args.destination.resolve()
        require(destination != bundle.resolve(), 'destination must differ from the build folder')
        destination.mkdir(parents=True, exist_ok=True)
        for name in names:
            shutil.copyfile(bundle / name, destination / name)
            require(digest(bundle / name) == digest(destination / name), f'copy verification failed: {name}')
        for index in range(count + 1, 5):
            stale = destination / f'havocmd-{index}.reu'
            if stale.exists():
                stale.unlink()
                print(f'Removed stale {stale}')
        print(f'Copied and SHA-256 verified {len(names)} files in {destination}')
    print(f'Finalized {len(names)} runtime files and {archive.name} ({archive.stat().st_size} bytes); '
          'hardware validation remains pending.')


if __name__ == '__main__':
    main()
