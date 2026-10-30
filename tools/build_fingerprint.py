"""Bind a local C64 build and its offline tests to their exact input files."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PRODUCTION_PROFILE = {'profile': 'stereo16k3', 'sample_rate': 16000,
                      'channels': 2, 'output_bits_per_sample': 8,
                      'full_song_count': 21, 'panorama_count': 21}
BUILD_INPUT_SUFFIXES = {'.c', '.h', '.S', '.inc', '.ld', '.rs', '.toml', '.lock'}
TEST_INPUT_SUFFIXES = BUILD_INPUT_SUFFIXES | {
    '.rs', '.py', '.ps1', '.sh', '.cfg', '.json', '.txt', '.md', '.toml', '.lock'}
PACKAGE_INPUTS = ['havocmd.prg', 'havocmd.prg.elf',
                  'musicdisk-manifest.json', 'music-provenance.json',
                  'LICENSE.txt', 'MUSIC-LICENSE.txt', 'build-inputs.json']
RUST_COMPONENTS = ('audio', 'panorama', 'disk', 'controller')
BANK_GATE_NAMES = ('pano_picture', 'controller_adjacent', 'pano_retain', 'pano_strip')
RUST_REPORT_FORMAT = 'havoc-rust-player-tests-v1'
COMPONENT_REPORTS = {
    'audio-player-tests.json': 'target-c64/build/audio-player-tests.json',
    'audio-fast-tests.json': 'target/audio-fast-test/audio-fast-tests.json',
    'tilt-tests.json': 'target/tilt-test/tilt-tests.json',
    'timer-tests.json': 'target-c64/build/timer-tests.json',
    'startup-tests.json': 'dist/havocmd/startup-tests.json',
    'rust-player-tests.json': 'target/rust-player-tests.json',
    'memory-tests.json': 'target/memory-test/memory-tests.json',
    'bank-tests.json': 'target/bank-test/bank-tests.json',
    'pano-copy-tests.json': 'target/pano-copy-test/pano-copy-tests.json',
    'rust-layout-tests.json': 'target-c64/build/rust-layout-tests.json',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def eligible(path):
    return path.is_file() and not any(part in ('build', 'target', '__pycache__')
                                     for part in path.relative_to(ROOT).parts)


def snapshot(paths):
    return {path.relative_to(ROOT).as_posix(): digest(path) for path in sorted(set(paths))}


def build_sources():
    paths = [path for path in (ROOT / 'target-c64').rglob('*')
             if eligible(path) and path.suffix in BUILD_INPUT_SUFFIXES]
    paths.extend(ROOT / name for name in ('tools/generate_tilt.py', 'tools/build_fingerprint.py',
                                         'scripts/build-musicdisk.sh', 'scripts/env.sh'))
    return snapshot(paths)


def test_sources():
    paths = [path for directory in ('target-c64', 'src', 'tools', 'scripts')
             for path in (ROOT / directory).rglob('*')
             if eligible(path) and path.suffix in TEST_INPUT_SUFFIXES]
    paths.extend(path for path in (ROOT / 'assets').rglob('*') if path.is_file())
    paths.extend(ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'LICENSE', 'README.md'))
    return snapshot(paths)


def package_volumes():
    """The volume files the content manifest names: one to four consecutive 16 MiB images."""
    content = json.loads((ROOT / 'dist/havocmd/musicdisk-manifest.json').read_text())
    names = [volume['file'] for volume in content.get('volumes', [])]
    require(1 <= len(names) <= 4 and names == [f'havocmd-{i}.reu' for i in range(1, len(names) + 1)],
            'content manifest must name one to four consecutive volume files')
    return names


def package_inputs():
    return snapshot(ROOT / 'dist/havocmd' / name for name in PACKAGE_INPUTS + package_volumes())


def verify_content_profile():
    content = json.loads((ROOT / 'dist/havocmd/musicdisk-manifest.json').read_text())
    package_volumes()
    require(content.get('content_verified') is True and
            all(content.get(key) == value for key, value in PRODUCTION_PROFILE.items()),
            'production content must be verified full-song stereo16k3 (16000 Hz, 2 channels, 8-bit output)')
    tracks = [entry for entry in content.get('entries', []) if entry.get('kind') == 'track']
    panoramas = [entry for entry in content.get('entries', []) if entry.get('kind') == 'panorama']
    require(len(tracks) == len(panoramas) == 21 and
            {entry['id'] for entry in tracks} == {entry['id'] for entry in panoramas} == set(range(21)) and
            all(len(entry.get('metadata', [])) == 4 and entry['metadata'][0] == 16000 and
                entry['metadata'][1] > 0 and entry['metadata'][2:] == [2, 3] for entry in tracks),
            'production directory must contain 21 complete stereo16k3 tracks and paired panoramas')
    return content


def rust_test_sources():
    paths = list((ROOT / 'target-c64/src').rglob('*.rs'))
    paths.extend(ROOT / f'tools/test-{name}-rust.rs' for name in RUST_COMPONENTS)
    paths.extend(ROOT / name for name in ('tools/test-rust-player.py', 'tools/build_fingerprint.py',
                                         'tools/prepare-musicdisk.py', 'assets/music/catalog.json',
                                         'target-c64/include/musicdisk_assets.h'))
    return snapshot(paths)


def verify_rust_tests(content=None):
    path = ROOT / 'target/rust-player-tests.json'
    require(path.is_file(), 'missing mandatory Rust component test report')
    report = json.loads(path.read_text())
    require(report.get('format') == RUST_REPORT_FORMAT and report.get('application_language') == 'Rust' and
            report.get('passed') is True and report.get('overflow_checks') is True,
            'Rust component tests must pass with overflow checks')
    cases = report.get('cases', [])
    require(len(cases) == len(RUST_COMPONENTS) and
            {case.get('component') for case in cases} == set(RUST_COMPONENTS) and
            all(case.get('passed') is True and case.get('tests_passed', 0) > 0 and
                case.get('tests_failed') == case.get('tests_ignored') == case.get('tests_filtered') == 0
                for case in cases), 'all four Rust component models must run without skipped tests')
    require(report.get('source_sha256') == rust_test_sources(), 'Rust component sources changed after testing')
    if content is None:
        content = verify_content_profile()
    entry = next((entry for entry in content.get('entries', [])
                  if entry.get('kind') == 'track' and entry.get('id') == 1), None)
    fixture = report.get('fixture', {})
    require(entry is not None and fixture.get('id') == 1 and fixture.get('name') == 'hammond-rocker' and
            fixture.get('profile') == 'stereo16k3' and fixture.get('sample_rate') == 16000 and
            fixture.get('frames', 0) > 80000 and fixture.get('frames') == entry['metadata'][1] and
            fixture.get('bytes') == entry.get('length') and fixture.get('sha256') == entry.get('sha256') and
            fixture.get('decoded_pcm_bytes_checked', 0) >= 160000,
            'Rust audio test must use the authenticated full16 Hammond asset from this package')
    return report


def verify_rust_layout():
    path = ROOT / COMPONENT_REPORTS['rust-layout-tests.json']
    require(path.is_file(), 'missing mandatory Rust layout audit')
    report = json.loads(path.read_text())
    require(report.get('rust_layout_passed') is True and
            report.get('overlay_audit', {}).get('seed_dma_after_initialize') is True and
            report.get('overlay_audit', {}).get('rotation_sections_absent') is True and
            report.get('bank_audit', {}).get('passed') is True and
            report.get('bank_audit', {}).get('gate_count') == len(BANK_GATE_NAMES) and
            report.get('bank_audit', {}).get('direct_mmio_rejected') is True and
            report.get('bank_audit', {}).get('nested_gates_rejected') is True,
            'Rust resident/startup ownership audit must pass')
    require(report.get('elf_sha256') == digest(ROOT / 'dist/havocmd/havocmd.prg.elf'),
            'Rust layout audit does not match the production ELF')
    return report


def verify_build():
    bundle = ROOT / 'dist/havocmd'
    report = json.loads((bundle / 'build-inputs.json').read_text())
    require(report.get('format') == 'havoc-c64-build-inputs-v1', 'missing successful C64 build stamp')
    require(report.get('audio_profile') == PRODUCTION_PROFILE, 'C64 build uses a different production audio profile')
    require(report.get('application_language') == 'Rust' and report.get('rust_compiler_version') and
            len(report.get('rust_compiler_sha256', '')) == 64 and len(report.get('linker_compiler_sha256', '')) == 64,
            'production build must identify the Rust application compiler and assembly linker')
    spec = report.get('rust_target_spec', {})
    require(len(report.get('rust_target_sha256', '')) == 64 and
            spec.get('arch') == 'mos' and str(spec.get('target-pointer-width')) == '16' and
            str(spec.get('target-c-int-width')) == '16' and spec.get('vendor') == 'c64',
            'production build must record the MOS C64 target JSON and fingerprint')
    verify_content_profile()
    require(report['source_sha256'] == build_sources(), 'C64 sources changed after compilation; rebuild first')
    require(report['prg_sha256'] == digest(bundle / 'havocmd.prg'), 'PRG changed after compilation')
    require(report['elf_sha256'] == digest(bundle / 'havocmd.prg.elf'), 'ELF changed after compilation')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('begin', 'finish', 'verify'))
    parser.add_argument('--compiler', type=Path)
    parser.add_argument('--rust-compiler', type=Path)
    parser.add_argument('--rust-target', type=Path)
    args = parser.parse_args()
    pending = ROOT / 'target-c64/build/musicdisk-build-inputs.pending.json'
    destination = ROOT / 'dist/havocmd/build-inputs.json'
    if args.action == 'begin':
        require(args.compiler is not None, 'begin requires the actual compiler path')
        require(args.rust_compiler is not None, 'production build requires the actual Rust compiler path')
        require(args.rust_target is not None and args.rust_target.is_file(),
                'production build requires the actual Rust target JSON')
        destination.unlink(missing_ok=True)
        verify_content_profile()
        report = {'format': 'havoc-c64-build-inputs-v1', 'source_sha256': build_sources(),
                  'audio_profile': PRODUCTION_PROFILE,
                  'compiler_version': subprocess.check_output([str(args.compiler), '--version'], text=True).strip()}
        if args.rust_compiler:
            report.update({'application_language': 'Rust',
                           'rust_compiler_version': subprocess.check_output([str(args.rust_compiler), '-vV'], text=True).strip(),
                           'rust_compiler_sha256': digest(args.rust_compiler),
                           'linker_compiler_sha256': digest(args.compiler),
                           'rust_target_path': str(args.rust_target.resolve()),
                           'rust_target_sha256': digest(args.rust_target),
                           'rust_target_spec': json.loads(args.rust_target.read_text())})
        pending.parent.mkdir(parents=True, exist_ok=True)
        pending.write_text(json.dumps(report, indent=2)+'\n')
    elif args.action == 'finish':
        report = json.loads(pending.read_text())
        require(report.get('application_language') == 'Rust', 'production build must compile a Rust application')
        require(digest(Path(report['rust_target_path'])) == report['rust_target_sha256'],
                'Rust target JSON changed while compiler was running')
        require(report.get('audio_profile') == PRODUCTION_PROFILE, 'production audio profile changed while compiling')
        verify_content_profile()
        require(report['source_sha256'] == build_sources(), 'C64 input changed while compiler was running')
        report['prg_sha256'] = digest(ROOT / 'dist/havocmd/havocmd.prg')
        report['elf_sha256'] = digest(ROOT / 'dist/havocmd/havocmd.prg.elf')
        destination.write_text(json.dumps(report, indent=2)+'\n')
        pending.unlink()
    else:
        verify_build()
    print(f'C64 build fingerprint: {args.action} passed.')


if __name__ == '__main__':
    main()
