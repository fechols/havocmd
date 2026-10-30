"""Boot the production PRG from dirty RAM against the actual volumes.

This is a local instruction/peripheral model, never a hardware timing claim.
UCI reads real files; sampler completion is accelerated and DMA costs one
modeled PHI2 per byte. The beam clock includes both peripheral waits and D031.
"""
import argparse
import hashlib
import importlib.util
import json
import struct
import time
from pathlib import Path

from native_cpu import I, U

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('prototype_tests', ROOT/'tools/test-prototype.py')
prototype = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prototype)


def le(data, at, count=4):
    return int.from_bytes(data[at:at+count], 'little')


def asset(volumes, track, kind):
    entry = volumes[0][64+(track+(21 if kind == 2 else 0))*64:][:64]
    assert le(entry, 0, 2) == track and entry[2] == kind
    volume, offset, remaining = entry[3], le(entry, 4), le(entry, 8)
    result = bytearray()
    while remaining:
        count = min(remaining, len(volumes[volume])-offset)
        assert count > 0
        result.extend(volumes[volume][offset:offset+count])
        remaining -= count
        volume, offset = volume+1, 4096
    return bytes(result)


def pcm_prefix(blob, count):
    """Independent bit reader, predictor saturation and signed8 rounding."""
    frames, books = le(blob, 8), le(blob, 18, 2)
    assert blob[:4] == b'DLT5' and le(blob, 4) in (16000, 24000) and blob[20:22] == b'\x03\x02'
    assert count <= frames, 'diagnostic track must be longer than the tested PCM prefix'
    groups = frames//4
    short, long = groups//books*4, groups % books
    boundary, stream = long*(short+4), 32+books*32
    predictors, output = [0, 0], bytearray()
    for frame in range(count):
        book = frame//(short+4) if frame < boundary else long+(frame-boundary)//short
        for channel in range(2):
            bit = (frame*2+channel)*3
            code = sum(((blob[stream+(bit+s)//8] >> ((bit+s) % 8)) & 1) << s for s in range(3))
            delta = struct.unpack_from('<h', blob, 32+book*32+channel*16+code*2)[0]
            predictors[channel] = max(-32768, min(32767, predictors[channel]+delta))
            output.append(min(127, (predictors[channel]+128) >> 8) & 255)
    return bytes(output)


class Dos:
    """Read-only Ultimate DOS, including 512-byte response packets."""
    def __init__(self, volumes, reu, absent=False, corrupt=False):
        self.volumes, self.reu = volumes, reu
        self.absent, self.corrupt = absent, corrupt
        self.wire = bytearray()
        self.opened, self.position = None, 0
        self.answer = self.status = b''
        self.at = self.end = self.status_at = self.state = 0
        self.commands, self.paths, self.loads, self.header_reads = [], [], [], []

    def reply(self, data=b'', status=b'00,OK'):
        self.answer, self.status = data, status
        self.at = self.status_at = 0
        self.end = min(len(data), 512)
        self.state = 0x30 if self.end < len(data) else 0x20

    def process(self):
        wire = bytes(self.wire)
        self.wire.clear()
        assert len(wire) >= 2 and wire[0] == 1, wire
        opcode = wire[1]
        self.commands.append(opcode)
        if opcode in (0x12, 0x17):
            self.reply(b'/')
        elif opcode == 0x11:
            assert wire == b'\x01\x11/'
            self.reply()
        elif opcode == 0x13:
            self.reply()
        elif opcode == 0x14:
            self.reply(b'\x10USB11')
        elif opcode == 2:
            assert wire[2] == 1, 'DOS open must be read-only'
            path = wire[3:].decode('ascii')
            self.paths.append(path)
            choices = [f'/USB11/havocmd/havocmd-{n}.reu' for n in range(1, len(self.volumes) + 1)]
            self.opened = choices.index(path) if path in choices else None
            self.position = 0
            self.reply(status=b'00,OK' if self.opened is not None else b'62,FILE NOT FOUND')
        elif opcode == 3:
            self.opened = None
            self.reply()
        elif opcode == 7:
            assert self.opened is not None
            self.reply(len(self.volumes[self.opened]).to_bytes(4, 'little'))
        elif opcode == 4:
            assert self.opened is not None and wire[2:] == b'\x00\x10'
            assert self.position == 0
            data = bytearray(self.volumes[self.opened][:4096])
            self.header_reads.append(self.opened)
            if self.corrupt and self.opened == len(self.volumes) - 1:
                data[80] ^= 1
            self.position += len(data)
            self.reply(bytes(data), status=b'')
        elif opcode == 6:
            assert self.opened is not None and len(wire) == 6
            self.position = le(wire, 2)
            self.reply()
        elif opcode == 0x21:
            assert self.opened is not None and len(wire) == 10
            destination, count = le(wire, 2), le(wire, 6)
            assert 0 < count <= 16384 and self.position >= 4096
            assert self.position+count <= len(self.volumes[self.opened])
            assert 0x1000 <= destination and destination+count <= 0xa01000
            data = self.volumes[self.opened][self.position:self.position+count]
            self.reu[destination:destination+count] = data
            self.loads.append((self.opened, self.position, destination, count))
            self.position += count
            self.reply(f'${count:6x} BYTES LOADED TO REU ${destination:6x}'.encode('ascii'))
        else:
            raise AssertionError(f'unexpected DOS command ${opcode:02x}')

    def read(self, address):
        if address == 0xdf1d:
            return 0xff if self.absent else 0xc9
        if address == 0xdf1c:
            return self.state | (0x80 if self.at < self.end else 0) | (
                0x40 if self.state == 0x20 and self.status_at < len(self.status) else 0)
        if address == 0xdf1e:
            assert self.at < self.end
            value = self.answer[self.at]
            self.at += 1
            return value
        assert address == 0xdf1f and self.status_at < len(self.status)
        value = self.status[self.status_at]
        self.status_at += 1
        return value

    def write(self, address, value):
        if address == 0xdf1d:
            assert self.state == 0 and len(self.wire) < 112
            self.wire.append(value)
            return
        assert address == 0xdf1c
        if value == 1:
            self.process()
            return
        assert value == 2 and self.state >= 0x20
        assert self.at == self.end
        if self.state == 0x20:
            assert self.status_at == len(self.status)
        if self.end < len(self.answer):
            self.end = min(len(self.answer), self.end+512)
            self.state = 0x30 if self.end < len(self.answer) else 0x20
        else:
            self.state = 0


def signed8(value):
    return value-256 if value & 128 else value


class CameraQuality:
    """Observe real rendered poses without driving or bypassing cache readiness."""
    def __init__(self):
        self.first = self.previous = None
        self.frames = self.moving_frames = self.stationary_frames = 0
        self.current_hold = self.maximum_hold = 0
        self.current_hold_x = self.current_hold_y = 0
        self.maximum_hold_x = self.maximum_hold_y = 0
        self.distance_x = self.distance_y = 0
        self.maximum_step = 0

    def observe(self, x, y, k):
        assert k == 0, 'production panorama must remain level'
        self.frames += 1
        if self.previous is None:
            self.first = self.previous = (x,y)
            return
        dx, dy = abs(x-self.previous[0]), abs(y-self.previous[1])
        assert max(dx,dy) <= 1, ('camera jumped by more than one pixel',self.frames,self.previous,(x,y))
        self.maximum_step = max(self.maximum_step,dx,dy)
        self.distance_x += dx; self.distance_y += dy
        self.current_hold_x = 0 if dx else self.current_hold_x+1
        self.current_hold_y = 0 if dy else self.current_hold_y+1
        self.maximum_hold_x = max(self.maximum_hold_x,self.current_hold_x)
        self.maximum_hold_y = max(self.maximum_hold_y,self.current_hold_y)
        if dx or dy:
            self.moving_frames += 1
            self.current_hold = 0
        else:
            self.stationary_frames += 1
            self.current_hold += 1
            self.maximum_hold = max(self.maximum_hold,self.current_hold)
        self.previous = (x,y)

    def report(self):
        minimum_distance = self.frames*95//200
        passed = self.distance_x >= minimum_distance and self.distance_y >= minimum_distance and max(self.maximum_hold,self.maximum_hold_x,self.maximum_hold_y) <= 4
        return {'passed':passed, 'observed_frames':self.frames, 'first_pose':self.first, 'last_pose':self.previous,
                'distance_x':self.distance_x, 'distance_y':self.distance_y,
                'moving_frames':self.moving_frames, 'stationary_frames':self.stationary_frames,
                'maximum_consecutive_stationary_frames':self.maximum_hold,
                'maximum_stationary_x_frames':self.maximum_hold_x,
                'maximum_stationary_y_frames':self.maximum_hold_y,
                'unexpected_hold_frames':max(0,(self.frames-1)//2-self.moving_frames),
                'unexpected_x_holds':max(0,(self.frames-1)//2-self.distance_x),
                'unexpected_y_holds':max(0,(self.frames-1)//2-self.distance_y),
                'maximum_frame_step':self.maximum_step,
                'minimum_axis_distance':minimum_distance, 'maximum_allowed_hold':4,
                'automatic_request_period_frames':2, 'input_injection':False}


class CallProfiler:
    """Optional inclusive PHI2 costs from real entry/RTS stack boundaries."""
    def __init__(self, symbols):
        self.entries = {symbols[name]:name for name in
                        ('pano_step','pano_tick')}
        self.active, self.calls = [], []

    def observe(self, machine):
        while self.active and machine.pc == self.active[-1]['return'] and machine.s == self.active[-1]['return_s']:
            call = self.active.pop()
            self.calls.append({'function':call['function'],
                               'ticks':round((machine.event_units()-call['start_units'])/machine.CPU_PER_PHI2,2),
                               'cpu_cycles':machine.cycles-call['cycles'],
                               'result_a':machine.a})
        if machine.pc in self.entries:
            address = (machine.mem[0x100+((machine.s+1)&255)] |
                       machine.mem[0x100+((machine.s+2)&255)] << 8)
            self.active.append({'function':self.entries[machine.pc], 'return':(address+1)&65535,
                                'return_s':(machine.s+2)&255,
                                'start_units':machine.event_units(), 'cycles':machine.cycles})

    def take_calls(self):
        calls, self.calls = self.calls, []
        return calls


class MusicdiskMachine(prototype.BeamMachine):
    def __init__(self, prg, expected, symbols, volumes, fault=None, pcm_frames=53248, profile=False):
        memory = bytearray((address*73+0xa5) & 255 for address in range(65536))
        load = le(prg, 0, 2)
        assert load+len(prg)-2 <= 65536
        memory[load:load+len(prg)-2] = prg[2:]
        memory[0:2] = b'\x2f\x37'
        super().__init__(memory, initial_line=100)
        self.expected, self.symbols, self.fault = expected, symbols, fault
        # The 6510 sees separate RAM beneath the I/O registers. Never let a
        # hidden-I/O store trigger VIC, CIA, sampler or REU behavior.
        self.ram_under_io = bytearray((i*61+0x97) & 255 for i in range(4096))
        self.banked_instructions = 0
        self.profiler = CallProfiler(symbols) if profile else None
        self.pc = prototype.basic_sys_entry(memory, load)
        assert self.pc == symbols['_start']
        self.p = U
        self.reu = bytearray([0xa5])*0x1000000
        self.dos = Dos(volumes, self.reu, absent=fault == 'missing_uci', corrupt=fault == 'bad_header')
        self.extra_units = 0
        self.measuring = self.completed = self.started = self.cache_loaded = False
        self.crt_checked = self.relocation_checked = False
        self.primed = self.prelaunch_writes = self.calibrations = 0
        self.stack_min = 0xd000
        self.frame_return = None
        self.frame_trace_start = 0
        self.service_return = None
        self.job_marker = 0
        self.job_events = []
        self.job_dma_bytes = 0
        self.job_new_book = False
        self.published_pcm_bytes = 0
        first_track = asset(volumes, 1, 1)
        self.sample_rate = le(first_track, 4)
        self.initial_divider = 6250000//self.sample_rate
        self.first_pcm = pcm_prefix(first_track, pcm_frames)
        self.first_art = asset(volumes, 1, 2)
        art_record = volumes[0][64+(1+21)*64:][:64]
        self.art_width, self.art_height, self.art_storage_width, self.art_storage_height = struct.unpack_from('<IIII', art_record, 16)

    def event_units(self):
        return super().event_units()+self.extra_units

    def value(self, name, count=1):
        return le(self.mem, self.symbols[name], count)

    def io_hidden(self):
        return self.mem[1] & 3 == 0

    def read(self, address):
        if 0xd000 <= address < 0xe000 and self.io_hidden():
            return self.ram_under_io[address-0xd000]
        if 0xdd04 <= address <= 0xdd07:
            ticks = self.event_units()//self.CPU_PER_PHI2
            return ((0xffffffff-ticks) >> ((address-0xdd04)*8)) & 255
        if 0xdf1c <= address <= 0xdf1f:
            return self.dos.read(address)
        if address == 0xdf21:
            return 0xff if self.fault == 'missing_sampler' else 0x10
        if address == 0xdf20:
            if self.measuring and not self.completed:
                frames = int.from_bytes(self.mem[0xdf29:0xdf2c], 'big')//2
                divider = int.from_bytes(self.mem[0xdf2e:0xdf30], 'big')
                assert frames in (4096, 16384)
                denominator = self.sample_rate*self.initial_divider
                ticks = (frames*1022727*divider+denominator//2)//denominator
                self.extra_units += ticks*self.CPU_PER_PHI2
                self.completed = True
                self.calibrations += 1
                return 0
            return int(self.completed)
        if address in (0xdc00, 0xdc01):
            return 255
        return super().read(address)

    def write(self, address, value):
        if 0xd000 <= address < 0xe000 and self.io_hidden():
            self.ram_under_io[address-0xd000] = value & 255
            self.writes.append(address)
            return
        old_value = self.mem[address]
        if 0x0200 <= address < 0x0800:
            assert self.p & I, f'low RAM write with IRQs enabled: ${address:04x}'
        if address == 0xd020 and not self.started:
            self.prelaunch_writes += 1
            if not self.fault:
                assert value & 15 == 0, 'colored startup frame before audio/picture ready'
        super().write(address, value)
        state_start = self.symbols['havoc_audio_chunk_state']
        if self.service_return is not None and state_start <= address < state_start+6:
            if (old_value, value) in ((3, 0), (0, 1)):
                self.job_events.append({'operation': 'drain' if value == 0 else 'fill',
                                        'slot': address-state_start,
                                        'ticks_with_loop_overhead': round((self.event_units()-self.job_marker)/63, 2),
                                        'dma_bytes': self.job_dma_bytes, 'new_book': self.job_new_book})
                self.job_marker = self.event_units()
                self.job_dma_bytes = 0
                self.job_new_book = False
        if 0xdf1c <= address <= 0xdf1d:
            self.dos.write(address, value)
        elif address == 0xdf01:
            ram, reu, count = le(self.mem, 0xdf02, 2), le(self.mem, 0xdf04, 3), le(self.mem, 0xdf07, 2) or 65536
            if self.service_return is not None:
                self.job_dma_bytes += count
                self.job_new_book |= value == 0x91 and ram == 0xe680
            assert ram+count <= 65536 and reu+count <= len(self.reu)
            if value == 0x91:
                if ram == 0x4000 and count == 32768:
                    assert self.relocation_checked and not 0x6000 <= self.pc < 0xc000
                    assert self.reu[reu:reu+count] == self.first_art[:32768]
                    self.cache_loaded = True
                self.mem[ram:ram+count] = self.reu[reu:reu+count]
            elif value == 0x90:
                if self.started and 0xfe0000 <= reu < 0xff0000:
                    assert reu == 0xfe0000+(self.published_pcm_bytes & 65535), 'non-sequential PCM ring destination'
                    wanted = self.first_pcm[self.published_pcm_bytes:self.published_pcm_bytes+count]
                    assert len(wanted) == count and self.mem[ram:ram+count] == wanted, (
                        'published PCM differs from original track', self.published_pcm_bytes, count)
                    self.published_pcm_bytes += count
                self.reu[reu:reu+count] = self.mem[ram:ram+count]
            else:
                raise AssertionError(f'unexpected REU DMA ${value:02x}')
            # With autoload disabled, real REU registers retain their advanced
            # addresses and exhausted count. The horizontal drain relies on it.
            assert self.mem[0xdf0a] == 0
            self.mem[0xdf02:0xdf04] = ((ram+count) & 65535).to_bytes(2, 'little')
            self.mem[0xdf04:0xdf07] = ((reu+count) & 0xffffff).to_bytes(3, 'little')
            self.mem[0xdf07:0xdf09] = bytes(2)
            self.extra_units += count*self.CPU_PER_PHI2
        elif address == 0xdf20:
            self.measuring, self.completed = value == 0x45, False
            if value == 0x45:
                assert self.mem[0xdf21] == self.mem[0xdf41] == 0, 'calibration must stay muted'
            if value == 0x43:
                assert self.cache_loaded and self.relocation_checked
                self.primed = self.value('produced', 4)
                self.published_pcm_bytes = self.primed*2
                assert 3968 <= self.primed < 4096, self.primed
                assert self.reu[0xfe0000:0xfe0000+self.primed*2] == self.first_pcm[:self.primed*2], 'first PCM differs from independent original-track decode'
                assert self.mem[0x4000:0xc000] == self.first_art[:32768]
        elif address == 0xdf40 and value == 0x43:
            assert self.primed >= 3968
            self.started = True

    def check_crt(self):
        for name in ('data', 'zp_data'):
            start, count = self.symbols[f'__{name}_start'], self.symbols[f'__{name}_size']
            assert self.mem[start:start+count] == self.expected[start:start+count], name
        for name in ('bss', 'zp_bss'):
            start, count = self.symbols[f'__{name}_start'], self.symbols[f'__{name}_size']
            assert not any(self.mem[start:start+count]), name
        assert self.p & I
        self.crt_checked = True

    def check_relocation(self):
        assert self.crt_checked
        for name in ('high', 'mid', 'tail'):
            start, end = self.symbols[f'__{name}_start'], self.symbols[f'__{name}_end']
            assert self.mem[start:end] == self.expected[start:end], f'{name} resident relocation'
        start, end = self.symbols['__bank_start'], self.symbols['__bank_end']
        assert self.ram_under_io[start-0xd000:end-0xd000] == self.expected[start:end], 'bank resident relocation'
        assert not self.io_hidden(), 'startup relocation did not restore I/O'
        self.relocation_checked = True

    def step(self):
        if self.profiler is not None and self.started:
            self.profiler.observe(self)
        if 0xd000 <= self.pc < 0xe000:
            assert self.io_hidden() and self.p & I, 'banked execution requires hidden I/O and disabled IRQs'
            self.banked_instructions += 1
        if self.pc in (self.symbols['havoc_raster_frame'], self.symbols['reu_execute']):
            assert not self.io_hidden(), 'raster or DMA entered with I/O hidden'
        if self.started and self.pc == self.symbols['audio_service']:
            self.service_return = ((self.mem[0x100+((self.s+1) & 255)] |
                                    self.mem[0x100+((self.s+2) & 255)] << 8)+1) & 65535
            self.job_marker = self.event_units()
            self.job_dma_bytes = 0
            self.job_new_book = False
        if self.pc == self.service_return:
            self.service_return = None
        if self.pc == self.symbols['main'] and not self.crt_checked:
            self.check_crt()
        if self.pc == self.symbols['boot_prepare'] and not self.relocation_checked:
            self.check_relocation()
        if self.cache_loaded:
            assert not 0x4000 <= self.pc < 0xc000, f'executing overwritten overlay/cache: ${self.pc:04x}'
        if self.crt_checked:
            stack = le(self.mem, 2, 2)
            self.stack_min = min(self.stack_min, stack)
            assert stack >= 0xcf80, f'software stack overlaps resident mid code: ${stack:04x}'
        if self.started and self.pc == self.symbols['havoc_raster_frame'] and self.frame_return is None:
            self.frame_return = ((self.mem[0x100+((self.s+1) & 255)] |
                                  self.mem[0x100+((self.s+2) & 255)] << 8)+1) & 65535
            self.frame_trace_start = len(self.beam_border)
        super().step()


def boot(prg, expected, symbols, volumes, fault=None, limit=None, row_pixels=202, frames=600, profile=False):
    print(f'Booting local production model: {fault or "production_start"}', flush=True)
    machine = MusicdiskMachine(prg, expected, symbols, volumes, fault, 0 if fault else 32768+512*frames, profile=profile)
    limit = limit or max(80_000_000, 20_000_000 + frames*400_000)
    camera_quality = CameraQuality()
    begin = time.monotonic()
    completed_frames, frame_starts, tilt_values = 0, [], []
    foreground_trace = []
    last_frame_end = 0
    report = None
    for instructions in range(1, limit+1):
        if completed_frames and machine.pc == symbols['havoc_raster_frame'] and machine.frame_return is None:
            state = {'frame': completed_frames, 'produced': machine.value('produced', 4),
                     'consumed': machine.value('consumed', 4),
                     'lead': machine.value('produced', 4)-machine.value('consumed', 4),
                     'chunk_state': list(machine.mem[symbols['havoc_audio_chunk_state']:][:6]),
                     'chunk_units': list(machine.mem[symbols['havoc_audio_chunk_units']:][:6]),
                     'foreground_ticks': round((machine.event_units()-last_frame_end)/63, 2),
                     'ticks_before_next_top': round(((frame_starts[-1]+263)*machine.UNITS_PER_LINE-machine.event_units())/63, 2),
                     'jobs': machine.job_events[:]}
            machine.job_events.clear()
            if machine.profiler is not None:
                state['profile_calls'] = machine.profiler.take_calls()
            for name, size in [('drain_cost', 2), ('fill_cost', 2), ('book_fill_cost', 2), ('havoc_dma_ticks', 2),
                               ('pano_max_job_ticks', 2),
                               ('fill_index', 1), ('drain_index', 1), ('planned', 4),
                               ('havoc_audio_quota', 1)]:
                if name in symbols:
                    state[name] = machine.value(name, size)
            state['camera'] = {'x':machine.value('pano_camera_x',2), 'y':machine.value('pano_camera_y',2)}
            foreground_trace.append(state)
        if completed_frames == frames and machine.pc == symbols['havoc_raster_frame']:
            # Include the foreground audio/loader/camera work after the final
            # picture, stopping only when the next picture is about to begin.
            assert not machine.value('havoc_fault'), ('audio fault', machine.value('havoc_fault'))
            assert machine.value('audio_running') == 1 and not machine.value('havoc_underruns', 2)
            report.update({'instructions': instructions, 'continuous_frames': completed_frames,
                           'frame_start_spacing_lines': sorted(set(b-a for a, b in zip(frame_starts, frame_starts[1:]))),
                           'software_stack_min': machine.stack_min,
                           'minimum_audio_lead_frames': machine.value('havoc_minimum_lead', 2),
                           'produced_frames': machine.value('produced', 4),
                           'consumed_frames': machine.value('consumed', 4),
                           'late_frames': machine.value('havoc_late_frames', 2),
                           'late_lines': machine.value('havoc_late_lines_total', 2),
                           'tilt_values': sorted(set(tilt_values)),
                           'uci_load_commands_after_frames': len(machine.dos.loads),
                           'total_original_pcm_bytes_checked': machine.published_pcm_bytes,
                           'foreground_trace': foreground_trace,
                           'banked_instructions': machine.banked_instructions,
                           'io_mapping_model': 'separate RAM beneath I/O; raster and DMA require visible I/O'})
            # One queue's worth of startup/segment granularity is acceptable;
            # a large initial ring must not conceal a sustained producer deficit.
            sustained = report['produced_frames']-machine.primed+1024 >= report['consumed_frames']
            report['sustained_audio_model_passed'] = sustained
            report['raster_deadlines_passed'] = report['late_frames'] == report['late_lines'] == 0
            report['camera_scroll'] = camera_quality.report()
            report['rotation_enabled'] = False
            report['all_frame_pixels_checked'] = completed_frames*240*row_pixels
            report['original_art_pixels_checked'] = completed_frames*240*200
            report['passed'] = sustained and report['raster_deadlines_passed'] and report['camera_scroll']['passed']
            break
        machine.step()
        if fault and machine.mem[0xd020] == (4 if fault == 'missing_sampler' else 2):
            assert not machine.started and not machine.primed
            assert machine.crt_checked and machine.relocation_checked
            if fault == 'missing_uci':
                assert machine.value('disk_error') == 1 and not machine.dos.commands
            elif fault == 'bad_header':
                assert machine.value('disk_error') == 3 and machine.dos.header_reads == list(range(len(volumes)))
            else:
                assert machine.value('disk_error') == 0 and machine.dos.header_reads == list(range(len(volumes)))
            report = {'case': fault, 'passed': True, 'instructions': instructions,
                      'border_color': machine.mem[0xd020], 'disk_error': machine.value('disk_error'),
                      'software_stack_min': machine.stack_min}
            break
        if machine.frame_return is not None and machine.pc == machine.frame_return:
            assert not fault and machine.value('audio_running') == 1 and not machine.value('havoc_fault'), (
                completed_frames, machine.value('havoc_fault'), machine.value('produced', 4), machine.value('consumed', 4))
            trace = machine.beam_border[machine.frame_trace_start:]
            row_writes = row_pixels+1
            assert len(trace) == 240*row_writes, len(trace)
            x, y = machine.value('havoc_raster_x'), machine.value('havoc_raster_y')
            world_x, world_y = machine.value('pano_camera_x', 2), machine.value('pano_camera_y', 2)
            first_line = trace[0][0]//machine.UNITS_PER_LINE
            assert first_line % 263 == machine.value('havoc_raster_top'), (
                'raster started late', completed_frames, first_line % 263,
                machine.value('havoc_raster_top'), foreground_trace[-1:] )
            frame_starts.append(first_line)
            if len(frame_starts) > 1:
                assert frame_starts[-1]-frame_starts[-2] == 263, ('skipped refresh', completed_frames, frame_starts[-2:])
            k = signed8(machine.value('havoc_tilt_k'))
            assert k == 0, ('unexpected production rotation', k)
            camera_quality.observe(world_x,world_y,k)
            tilt_values.append(k)
            if not completed_frames:
                assert k == 0, 'first picture must be level'
            for row in range(240):
                writes = trace[row*row_writes:(row+1)*row_writes]
                assert all(t//machine.UNITS_PER_LINE == first_line+row for t, _, _ in writes), (
                    'raster row escaped scanline', {'frame':completed_frames, 'k':k, 'row':row,
                    'expected_raster_line':(first_line+row)%263,
                    'first_write_tick':round((writes[0][0]-(first_line+row)*machine.UNITS_PER_LINE)/machine.CPU_PER_PHI2,3),
                    'last_write_tick':round((writes[-1][0]-(first_line+row)*machine.UNITS_PER_LINE)/machine.CPU_PER_PHI2,3),
                    'first_write_raster_line':(writes[0][0]//machine.UNITS_PER_LINE)%263,
                    'last_write_raster_line':(writes[-1][0]//machine.UNITS_PER_LINE)%263,
                    'foreground':foreground_trace[-1:]})
                wanted = []
                if row_pixels == 202:
                    # Independent integer DDA geometry, including both wrapped
                    # cache axes and the extra emitted pair outside the viewport.
                    shear = ((120-row)*k)//656
                    first_column = ((x + shear) & 255)//2
                    first_world_x = ((world_x+shear)//2)*2
                    base_y = y+row-(50*k+64)//128
                    prefix = (first_column*k)//128
                    for pair in range(101):
                        column = first_column+pair
                        source_y = (base_y+(column*k)//128-prefix) & 255
                        offset = (column & 127)*256+source_y
                        packed = machine.mem[0x4000+offset]
                        # Independently address the original art tiles. A live
                        # cache-only oracle would miss streaming corruption.
                        art_x = first_world_x+2*pair
                        art_y = world_y+row-(50*k+64)//128+(column*k)//128-prefix
                        # The two emitted guards lie outside the 200-pixel
                        # viewport and may already hold proactive strip data.
                        # Every visible nibble must still match original art.
                        mask = sum(15 << (4*half) for half in (0,1)
                                   if (world_x & 1) <= 2*pair+half < (world_x & 1)+200)
                        if mask:
                            assert 0 <= art_x < machine.art_storage_width and 0 <= art_y < machine.art_storage_height, ('source outside artwork storage', completed_frames, art_x, art_y)
                            tile = (art_x//16)*(machine.art_storage_height//8)+art_y//8
                            art_offset = 32768+tile*64+((art_x & 15)//2)*8+(art_y & 7)
                            assert packed & mask == machine.first_art[art_offset] & mask, ('panorama cache differs from original artwork', completed_frames, row, pair, art_x, art_y, packed, machine.first_art[art_offset],mask)
                        if not completed_frames:
                            assert packed == machine.first_art[offset], 'first picture seed differs'
                        wanted.extend((packed & 15, packed >> 4))
                else:
                    assert k == 0, 'legacy200-pixel renderer has no rotation oracle'
                    for column in range(row_pixels):
                        sx, sy = (x+column) & 255, (y+row) & 255
                        packed = machine.mem[0x4000+(sx//2)*256+sy]
                        wanted.append((packed >> (4*(sx & 1))) & 15)
                assert [color for _, color, _ in writes] == wanted+[0], ('renderer pixel mismatch', completed_frames, k, row)
            assert machine.dos.header_reads == list(range(len(volumes)))
            assert 0x12 in machine.dos.commands and 0x17 in machine.dos.commands and 0x14 in machine.dos.commands
            if not completed_frames:
                report = {'case': 'production_start', 'passed': True, 'instructions': instructions,
                      'sample_rate_hz': machine.sample_rate,
                      'sampler_ticks_per_ring': machine.value('sampler_ticks_per_ring', 4),
                      'primed_frames_before_dac': machine.primed, 'original_pcm_bytes_checked': machine.primed*2,
                      'prelaunch_black_writes': machine.prelaunch_writes, 'first_frame_rows': 240,
                      'first_frame_color_writes': len(trace), 'emitted_pixels_per_row': row_pixels,
                      'visible_pixels_per_row': 200, 'software_stack_min': machine.stack_min,
                      'uci_load_commands': len(machine.dos.loads), 'volume_headers_checked': machine.dos.header_reads,
                          'sampler_measurements': machine.calibrations, 'first_picture_k': k,
                          'resident_relocations_checked': ['high', 'mid', 'tail', 'bank'],
                          'bank_bytes_checked': symbols['__bank_end']-symbols['__bank_start']}
            completed_frames += 1
            if completed_frames % 60 == 0:
                progress = {'frame':completed_frames, 'k':k,
                            'lead':machine.value('produced',4)-machine.value('consumed',4),
                            'late_frames':machine.value('havoc_late_frames',2),
                            'late_lines':machine.value('havoc_late_lines_total',2),
                            'camera_x':world_x, 'camera_y':world_y,
                            'maximum_hold':camera_quality.maximum_hold,
                            'maximum_x_hold':camera_quality.maximum_hold_x,
                            'maximum_y_hold':camera_quality.maximum_hold_y,
                            'unexpected_holds':camera_quality.report()['unexpected_hold_frames'],
                            'loader_chunks':machine.value('disk_chunks',2)}
                print('Production model progress: '+json.dumps(progress), flush=True)
                if frames <= 600:
                    # This route stays inside the first panorama's edges.
                    # Enforce the final per-axis quality rules at each checkpoint
                    # so a known stall cannot waste the rest of a long CPU run.
                    quality = camera_quality.report()
                    assert quality['passed'], ('camera quality failed at checkpoint',
                                               progress, quality, foreground_trace[-1:])
            last_frame_end = machine.event_units()
            machine.frame_return = None
            machine.beam_border.clear()
            machine.border.clear()
            machine.writes.clear()
    else:
        raise AssertionError(f'boot timeout: pc=${machine.pc:04x}, disk={machine.value("disk_error")}, '
                             f'audio={machine.value("havoc_fault")}, produced={machine.value("produced", 4)}')
    report['elapsed_seconds'] = round(time.monotonic()-begin, 3)
    print(json.dumps({key: value for key, value in report.items() if key != 'foreground_trace'}), flush=True)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=ROOT/'dist/havocmd')
    parser.add_argument('--prg', type=Path)
    parser.add_argument('--elf', type=Path)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--row-pixels', type=int, choices=(200, 202), default=202,
                        help='emitted colors before final black; circular production kernel uses 202')
    parser.add_argument('--frames', type=int, default=600,
                        help='consecutive pictures and foreground iterations after startup (default: 600)')
    parser.add_argument('--profile', action='store_true',
                        help='record inclusive camera/cache call costs per foreground interval')
    parser.add_argument('--case', choices=('all', 'production_start', 'missing_uci', 'bad_header', 'missing_sampler'), default='all')
    args = parser.parse_args()
    assert args.frames >= 1
    prg_path = args.prg or args.directory/'havocmd.prg'
    elf_path = args.elf or prg_path.with_suffix(prg_path.suffix+'.elf')
    prg = prg_path.read_bytes()
    expected, symbols = prototype.image(elf_path)
    first = (args.directory/'havocmd-1.reu').read_bytes()
    assert 1 <= first[13] <= 4, 'volume 1 must record one to four volumes'
    volumes = [first] + [(args.directory/f'havocmd-{n}.reu').read_bytes() for n in range(2, first[13] + 1)]
    assert all(len(volume) == 0x1000000 for volume in volumes)
    cases = ['missing_uci', 'bad_header', 'missing_sampler', 'production_start'] if args.case == 'all' else [args.case]
    reports = [boot(prg, expected, symbols, volumes, None if case == 'production_start' else case,
                    row_pixels=args.row_pixels, frames=args.frames, profile=args.profile) for case in cases]
    report = {'machine_code_boot_passed': True,
              'continuous_model_passed': all(case['passed'] for case in reports), 'hardware_measured': False,
              'model': '63 CPU/PHI2 turbo, D031 1 MHz, 65 PHI2/line, 263 lines/frame; accelerated sampler; DMA 1 PHI2/byte; instantaneous read-only DOS',
              'prg_sha256': hashlib.sha256(prg).hexdigest(), 'volume_count': len(volumes),
              'volume_sha256': [hashlib.sha256(volume).hexdigest() for volume in volumes], 'cases': reports}
    destination = args.report or args.directory/'startup-tests.json'
    destination.write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    if not report['continuous_model_passed']:
        raise SystemExit('FAIL: sustained audio, raster deadlines or smooth camera coverage failed; see report')


if __name__ == '__main__':
    main()
