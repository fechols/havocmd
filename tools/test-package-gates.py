"""Regression tests for package hash gates, using isolated temporary fixtures."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import build_fingerprint as fingerprints

SOURCE_ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, SOURCE_ROOT/'tools'/filename)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


finalizer = module('package_finalizer_tests', 'finalize-musicdisk.py')
suite = module('package_suite_tests', 'test-musicdisk.py')
TESTS = ('panorama', 'disk-loader', 'camera-edges', 'package-gates', 'rust-layout', 'rust-player', 'audio-player', 'audio-fast', 'memory', 'bank', 'pano-copy', 'tilt', 'timer', 'musicdisk-startup')


class Gates(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='havoc-package-gates-')
        self.root = Path(self.temporary.name)
        self.previous = fingerprints.ROOT, finalizer.ROOT, suite.ROOT
        fingerprints.ROOT = finalizer.ROOT = suite.ROOT = self.root
        self.bundle = self.root/'dist/havocmd'
        for name in ('target-c64/main.c', 'tools/generate_tilt.py', 'tools/build_fingerprint.py',
                     'scripts/build-musicdisk.sh', 'scripts/env.sh', 'assets/musicdisk.cfg', 'assets/music/catalog.json',
                     'target-c64/src/main.rs', 'target-c64/include/musicdisk_assets.h',
                     'tools/test-rust-player.py', 'tools/prepare-musicdisk.py',
                     'tools/test-audio-rust.rs', 'tools/test-panorama-rust.rs',
                     'tools/test-disk-rust.rs', 'tools/test-controller-rust.rs', 'tools/test-motion-rust.rs',
                     'Cargo.toml', 'Cargo.lock', 'LICENSE', 'README.md',
                     'target-c64/docs/musicdisk-readme.txt'):
            self.write(name, 'fixture\n')
        for name in ('havocmd.prg', 'havocmd.prg.elf', 'LICENSE.txt', 'MUSIC-LICENSE.txt',
                     'music-provenance.json'):
            self.write('dist/havocmd/'+name, 'fixture\n')
        volumes = []
        for i in range(1, 5):
            path = self.bundle/f'havocmd-{i}.reu'
            with path.open('wb') as output:
                output.seek(0x1000000-1)
                output.write(bytes([i]))
            volumes.append({'file': path.name, 'bytes': path.stat().st_size,
                            'sha256': fingerprints.digest(path)})
        self.document('dist/havocmd/musicdisk-manifest.json', {
            'content_verified': True, 'volumes': volumes, **fingerprints.PRODUCTION_PROFILE,
            'entries': [{'id': i, 'kind': kind, 'sha256': 'a'*64, 'length': 2842847,
                         'metadata': [16000, 3358936, 2, 3]
                        if kind == 'track' else [905, 1086, 912, 1088]}
                        for i in range(21) for kind in ('track', 'panorama')]})
        self.target_spec = {'arch': 'mos', 'target-pointer-width': '16', 'target-c-int-width': '16', 'vendor': 'c64'}
        self.document('toolchain/mos-c64-none.json', self.target_spec)
        self.document('dist/havocmd/build-inputs.json', {
            'format': 'havoc-c64-build-inputs-v1', 'source_sha256': fingerprints.build_sources(),
            'audio_profile': fingerprints.PRODUCTION_PROFILE, 'application_language': 'Rust',
            'rust_compiler_version': 'rustc fixture', 'rust_compiler_sha256': 'b'*64,
            'linker_compiler_sha256': 'c'*64, 'rust_target_spec': self.target_spec,
            'rust_target_path': str(self.root/'toolchain/mos-c64-none.json'),
            'rust_target_sha256': fingerprints.digest(self.root/'toolchain/mos-c64-none.json'),
            'prg_sha256': fingerprints.digest(self.bundle/'havocmd.prg'),
            'elf_sha256': fingerprints.digest(self.bundle/'havocmd.prg.elf')})
        self.document('dist/havocmd/startup-tests.json', {
            'machine_code_boot_passed': True, 'prg_sha256': fingerprints.digest(self.bundle/'havocmd.prg'),
            'volume_sha256': [v['sha256'] for v in volumes],
            'cases': [{'case': name, 'passed': True, 'continuous_frames': 600,
                       'first_picture_k': 0, 'rotation_enabled':False, 'tilt_values':[0],
                       'all_frame_pixels_checked': 600*240*202, 'original_art_pixels_checked':600*240*200,
                                   'bank_bytes_checked':2000, 'banked_instructions':100000, 'raster_deadlines_passed': True,
                       'camera_scroll': {'passed':True, 'input_injection':False, 'observed_frames':600,
                                         'distance_x':299, 'distance_y':299,
                                         'maximum_consecutive_stationary_frames':1, 'maximum_stationary_x_frames':1,
                                         'maximum_stationary_y_frames':1, 'maximum_frame_step':1}} for name in
                      ('production_start', 'missing_uci', 'bad_header', 'missing_sampler')]})
        self.document('target-c64/build/audio-player-tests.json', {'passed': True})
        self.document('target/audio-fast-test/audio-fast-tests.json', {'passed': True})
        self.document('target/tilt-test/tilt-tests.json', {'passed': True})
        self.document('target-c64/build/timer-tests.json', {'passed': True})
        self.document('target/memory-test/memory-tests.json', {'passed': True})
        self.document('target/bank-test/bank-tests.json', {'passed': True})
        self.document('target/pano-copy-test/pano-copy-tests.json', {'passed': True})
        self.document('target-c64/build/rust-layout-tests.json', {
            'rust_layout_passed': True, 'elf_sha256': fingerprints.digest(self.bundle/'havocmd.prg.elf'),
            'overlay_audit': {'seed_dma_after_initialize': True, 'rotation_sections_absent': True},
            'bank_audit': {'passed':True, 'gate_count':len(fingerprints.BANK_GATE_NAMES), 'direct_mmio_rejected':True, 'nested_gates_rejected':True}})
        self.document('target/rust-player-tests.json', {
            'format': fingerprints.RUST_REPORT_FORMAT, 'application_language': 'Rust',
            'passed': True, 'overflow_checks': True, 'source_sha256': fingerprints.rust_test_sources(),
            'fixture': {'id': 1, 'name': 'hammond-rocker', 'profile': 'stereo16k3', 'sample_rate': 16000,
                        'frames': 3358936, 'bytes': 2842847, 'sha256': 'a'*64,
                        'decoded_pcm_bytes_checked': 160000},
            'cases': [{'component': name, 'passed': True, 'tests_passed': 1, 'tests_failed': 0,
                       'tests_ignored': 0, 'tests_filtered': 0} for name in fingerprints.RUST_COMPONENTS]})
        self.refresh_tests()

    def tearDown(self):
        fingerprints.ROOT, finalizer.ROOT, suite.ROOT = self.previous
        self.temporary.cleanup()

    def write(self, name, value):
        path = self.root/name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)

    def document(self, name, value):
        self.write(name, json.dumps(value))

    def refresh_tests(self):
        self.document('dist/havocmd/offline-tests.json', {
            'application_language': 'Rust', 'offline_tests_passed': True,
            'prg_sha256': fingerprints.digest(self.bundle/'havocmd.prg'),
            'tests': [{'test': name, 'passed': True} for name in TESTS],
            'source_sha256': fingerprints.test_sources(), 'package_sha256': fingerprints.package_inputs(),
            'component_report_sha256': {name: fingerprints.digest(self.root/path)
                                       for name, path in fingerprints.COMPONENT_REPORTS.items()}})

    def finalize(self):
        with patch.object(sys, 'argv', ['finalize-musicdisk.py']), contextlib.redirect_stdout(io.StringIO()):
            finalizer.main()

    def test_unchanged_bundle_finalizes_with_exact_checksums(self):
        self.finalize()
        report = json.loads((self.bundle/'build-report.json').read_text())
        self.assertEqual((report['profile'], report['sample_rate'], report['channels'], report['output_bits']),
                         ('stereo16k3', 16000, 2, 8))
        self.assertEqual(report['application_language'], 'Rust')
        self.assertIn('rust-player-tests.json', report['files'])
        self.assertIn('memory-tests.json', report['files'])
        self.assertIn('pano-copy-tests.json', report['files'])
        for line in (self.bundle/'SHA256SUMS.txt').read_text().splitlines():
            expected, name = line.split('  ')
            self.assertEqual(expected, fingerprints.digest(self.bundle/name))

    def test_failed_rust_layout_is_rejected(self):
        report = json.loads((self.root/'target-c64/build/rust-layout-tests.json').read_text())
        report['rust_layout_passed'] = False
        self.document('target-c64/build/rust-layout-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'resident/startup ownership audit must pass'):
            self.finalize()

    def test_rust_layout_must_match_the_actual_elf(self):
        report = json.loads((self.root/'target-c64/build/rust-layout-tests.json').read_text())
        report['elf_sha256'] = 'e'*64
        self.document('target-c64/build/rust-layout-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'does not match the production ELF'):
            self.finalize()

    def test_missing_rust_target_spec_is_rejected(self):
        build = json.loads((self.bundle/'build-inputs.json').read_text())
        build.pop('rust_target_spec')
        self.document('dist/havocmd/build-inputs.json', build)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'MOS C64 target JSON'):
            self.finalize()

    def test_target_spec_survives_a_different_host_path(self):
        build = json.loads((self.bundle/'build-inputs.json').read_text())
        build['rust_target_path'] = '/unavailable/original-build-host/mos-c64-none.json'
        self.document('dist/havocmd/build-inputs.json', build)
        self.refresh_tests()
        self.finalize()

    def test_target_spec_mutation_during_compile_is_rejected(self):
        with contextlib.redirect_stdout(io.StringIO()):
            with patch.object(sys, 'argv', ['build_fingerprint.py', 'begin', '--compiler', sys.executable,
                                           '--rust-compiler', sys.executable,
                                           '--rust-target', str(self.root/'toolchain/mos-c64-none.json')]):
                fingerprints.main()
            self.document('toolchain/mos-c64-none.json', dict(self.target_spec, cpu='changed'))
            with patch.object(sys, 'argv', ['build_fingerprint.py', 'finish']):
                with self.assertRaisesRegex(ValueError, 'target JSON changed while compiler'):
                    fingerprints.main()

    def test_c_build_cannot_be_relabelled_as_rust(self):
        build = json.loads((self.bundle/'build-inputs.json').read_text())
        build.pop('application_language')
        self.document('dist/havocmd/build-inputs.json', build)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'must identify the Rust application compiler'):
            self.finalize()

    def test_rust_component_failure_is_not_hidden_by_suite_success(self):
        report = json.loads((self.root/'target/rust-player-tests.json').read_text())
        report['cases'][0]['passed'] = False
        self.document('target/rust-player-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'all four Rust component'):
            self.finalize()

    def test_rust_component_skips_are_rejected(self):
        report = json.loads((self.root/'target/rust-player-tests.json').read_text())
        report['cases'][0]['tests_ignored'] = 1
        self.document('target/rust-player-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'without skipped tests'):
            self.finalize()

    def test_volume_count_must_agree_between_content_and_startup(self):
        startup = json.loads((self.bundle/'startup-tests.json').read_text())
        startup['volume_sha256'] = startup['volume_sha256'][:3]
        self.document('dist/havocmd/startup-tests.json', startup)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'same one to four volumes'):
            self.finalize()

    def test_rust_models_require_overflow_checks(self):
        report = json.loads((self.root/'target/rust-player-tests.json').read_text())
        report['overflow_checks'] = False
        self.document('target/rust-player-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'with overflow checks'):
            self.finalize()

    def test_short_diagnostic_cannot_replace_full16_fixture(self):
        report = json.loads((self.root/'target/rust-player-tests.json').read_text())
        report['fixture']['frames'] = 32000
        self.document('target/rust-player-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'authenticated full16 Hammond'):
            self.finalize()

    def test_different_full_song_hash_is_rejected(self):
        report = json.loads((self.root/'target/rust-player-tests.json').read_text())
        report['fixture']['sha256'] = 'd'*64
        self.document('target/rust-player-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'authenticated full16 Hammond'):
            self.finalize()

    def test_camera_quality_counts_real_pose_changes_and_rejects_stalls(self):
        startup=module('camera_quality_tests','test-musicdisk-startup.py')
        quality=startup.CameraQuality()
        for frame in range(600): quality.observe(336+frame//2,416+frame//2,0)
        report=quality.report()
        self.assertTrue(report['passed'])
        self.assertEqual(report['unexpected_hold_frames'],0)
        self.assertEqual((report['unexpected_x_holds'],report['unexpected_y_holds']),(0,0))
        self.assertEqual((report['distance_x'],report['distance_y'],report['maximum_consecutive_stationary_frames']),(299,299,1))
        stalled=startup.CameraQuality()
        for frame in range(600): stalled.observe(336+frame//60,416+frame//60,0)
        self.assertFalse(stalled.report()['passed'])
        one_axis=startup.CameraQuality()
        for frame in range(600): one_axis.observe(336+frame//2,416+frame//60,0)
        self.assertEqual(one_axis.report()['maximum_consecutive_stationary_frames'],1)
        self.assertEqual(one_axis.report()['maximum_stationary_y_frames'],59)
        self.assertFalse(one_axis.report()['passed'])
        with self.assertRaisesRegex(AssertionError,'remain level'): quality.observe(635,715,1)
        with self.assertRaisesRegex(AssertionError,'more than one pixel'): quality.observe(640,715,0)

    def test_ram_under_io_has_independent_backing_and_instruction_fetch(self):
        startup = module('bank_mapping_tests', 'test-musicdisk-startup.py')
        machine = startup.MusicdiskMachine.__new__(startup.MusicdiskMachine)
        startup.prototype.BeamMachine.__init__(machine, bytearray(65536))
        machine.ram_under_io = bytearray([0xa5])*4096
        machine.extra_units = 0
        machine.started = True
        machine.service_return = None
        machine.symbols = {'havoc_audio_chunk_state':0x200}
        machine.mem[1] = 0x34
        machine.mem[0xd020] = 6
        for address, value in ((0xd020,5),(0xd031,0),(0xdf01,0x91),(0xdd04,123)):
            machine.write(address,value)
            self.assertEqual(machine.read(address),value)
        self.assertEqual(machine.mem[0xd020],6)
        self.assertFalse(machine.beam_border)
        self.assertEqual(machine.speed_factor,1)
        # Both opcodes and operands must come from hidden RAM, not I/O latches.
        machine.ram_under_io[0x100:0x105] = bytes([0xa9,7,0x8d,0x00,0x03])
        machine.pc=0xd100
        startup.prototype.BeamMachine.step(machine)
        startup.prototype.BeamMachine.step(machine)
        self.assertEqual(machine.mem[0x300],7)
        machine.mem[1]=0x35
        self.assertEqual(machine.read(0xd020),6)
        machine.write(0xd020,9)
        self.assertEqual(machine.beam_border[-1][1],9)
        self.assertEqual(machine.ram_under_io[0x20],5)

    def test_rotation_cannot_satisfy_level_scrolling_gate(self):
        report=json.loads((self.bundle/'startup-tests.json').read_text())
        report['cases'][0]['tilt_values']=[0,1]
        self.document('dist/havocmd/startup-tests.json',report); self.refresh_tests()
        with self.assertRaisesRegex(ValueError,'level scrolling'): self.finalize()

    def test_long_camera_hold_is_rejected(self):
        report=json.loads((self.bundle/'startup-tests.json').read_text())
        report['cases'][0]['camera_scroll']['maximum_consecutive_stationary_frames']=60
        self.document('dist/havocmd/startup-tests.json',report); self.refresh_tests()
        with self.assertRaisesRegex(ValueError,'no long holds'): self.finalize()

    def test_single_axis_hold_cannot_hide_behind_other_axis_motion(self):
        report=json.loads((self.bundle/'startup-tests.json').read_text())
        report['cases'][0]['camera_scroll']['maximum_stationary_y_frames']=60
        self.document('dist/havocmd/startup-tests.json',report); self.refresh_tests()
        with self.assertRaisesRegex(ValueError,'no long holds'): self.finalize()

    def test_insufficient_camera_progress_is_rejected(self):
        report=json.loads((self.bundle/'startup-tests.json').read_text())
        report['cases'][0]['camera_scroll']['distance_x']=284
        self.document('dist/havocmd/startup-tests.json',report); self.refresh_tests()
        with self.assertRaisesRegex(ValueError,'sustained camera movement'): self.finalize()

    def test_incomplete_pixel_check_is_rejected(self):
        report = json.loads((self.bundle/'startup-tests.json').read_text())
        report['cases'][0]['all_frame_pixels_checked'] = 240*202
        self.document('dist/havocmd/startup-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'all pixels and raster deadlines'):
            self.finalize()

    def test_599_continuous_frames_cannot_pass(self):
        report = json.loads((self.bundle/'startup-tests.json').read_text())
        report['cases'][0]['continuous_frames'] = 599
        self.document('dist/havocmd/startup-tests.json', report)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'at least600'):
            self.finalize()

    def test_old_audio_profile_cannot_be_relabelled_by_new_finalizer(self):
        content = json.loads((self.bundle/'musicdisk-manifest.json').read_text())
        content.update(profile='stereo24k3', sample_rate=24000)
        self.document('dist/havocmd/musicdisk-manifest.json', content)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'must be verified full-song stereo16k3'):
            self.finalize()

    def test_old_build_profile_is_rejected(self):
        build = json.loads((self.bundle/'build-inputs.json').read_text())
        build['audio_profile'] = dict(fingerprints.PRODUCTION_PROFILE, sample_rate=24000)
        self.document('dist/havocmd/build-inputs.json', build)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'different production audio profile'):
            self.finalize()

    def test_track_rate_must_match_production_profile(self):
        content = json.loads((self.bundle/'musicdisk-manifest.json').read_text())
        content['entries'][0]['metadata'][0] = 24000
        self.document('dist/havocmd/musicdisk-manifest.json', content)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'directory must contain 21 complete stereo16k3'):
            self.finalize()

    def test_configuration_change_invalidates_tests(self):
        self.write('assets/musicdisk.cfg', 'changed\n')
        with self.assertRaisesRegex(ValueError, 'configuration changed'):
            self.finalize()

    def test_source_change_cannot_be_relabelled_as_an_existing_build(self):
        self.write('target-c64/main.c', 'changed\n')
        self.refresh_tests()  # A new test snapshot alone cannot bless an old PRG.
        with self.assertRaisesRegex(ValueError, 'changed after compilation'):
            self.finalize()

    def test_stale_content_manifest_rejected_even_if_new_volume_was_boot_tested(self):
        path = self.bundle/'havocmd-1.reu'
        with path.open('r+b') as output:
            output.write(b'changed')
        startup = json.loads((self.bundle/'startup-tests.json').read_text())
        startup['volume_sha256'][0] = fingerprints.digest(path)
        self.document('dist/havocmd/startup-tests.json', startup)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'both validated content and startup'):
            self.finalize()

    def test_partial_startup_run_cannot_replace_complete_gate(self):
        startup = json.loads((self.bundle/'startup-tests.json').read_text())
        startup['cases'] = startup['cases'][1:]
        self.document('dist/havocmd/startup-tests.json', startup)
        self.refresh_tests()
        with self.assertRaisesRegex(ValueError, 'all faults'):
            self.finalize()

    def test_source_mutation_during_suite_is_rejected(self):
        def fake_run(command, **kwargs):
            if str(command[-1]).endswith('test-audio-player.py'):
                self.write('assets/musicdisk.cfg', 'changed during tests\n')
            return SimpleNamespace(stdout='modeled test passed\n')
        previous_cwd = Path.cwd()
        try:
            with patch.object(suite.subprocess, 'run', side_effect=fake_run), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, 'while tests were running'):
                    suite.main()
        finally:
            import os
            os.chdir(previous_cwd)

    def test_source_mutation_during_compile_is_rejected(self):
        with contextlib.redirect_stdout(io.StringIO()):
            with patch.object(sys, 'argv', ['build_fingerprint.py', 'begin', '--compiler', sys.executable, '--rust-compiler', sys.executable,
                                                  '--rust-target', str(self.root/'toolchain/mos-c64-none.json')]):
                fingerprints.main()
            self.write('target-c64/main.c', 'changed during compile\n')
            with patch.object(sys, 'argv', ['build_fingerprint.py', 'finish']):
                with self.assertRaisesRegex(ValueError, 'compiler was running'):
                    fingerprints.main()
        self.assertFalse((self.bundle/'build-inputs.json').exists())


if __name__ == '__main__':
    unittest.main()
