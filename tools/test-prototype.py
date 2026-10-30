"""Execute linked NMOS6502 kernels; no physical C64U timing is claimed."""
from pathlib import Path
import argparse
import json
import hashlib
import random
import struct
import sys
from native_cpu import Cpu, I, U

ROOT = Path(__file__).resolve().parents[1]


def image(path):
    data = path.read_bytes()
    assert data[:6] == b'\x7fELF\x01\x01'
    off = struct.unpack_from('<I', data, 32)[0]
    size, count = struct.unpack_from('<HH', data, 46)
    sections = [struct.unpack_from('<10I', data, off + i * size) for i in range(count)]
    memory = bytearray(65536)
    symbols = {}
    for s in sections:
        _, kind, flags, address, start, length, link, _, _, entry = s
        if flags & 2 and kind == 1 and address + length <= 65536:
            memory[address:address+length] = data[start:start+length]
        if kind == 2:
            strings = sections[link]
            table = data[strings[4]:strings[4]+strings[5]]
            for at in range(start, start+length, entry):
                name, value, _, _, _, _ = struct.unpack_from('<IIIBBH', data, at)
                end = table.find(b'\0', name)
                symbols[table[name:end].decode()] = value
    return memory, symbols


class Machine(Cpu):
    def __init__(self, memory):
        super().__init__(memory, [(0, 65536)])
        self.border = []

    def write(self, address, value):
        super().write(address, value)
        if address == 0xd020:
            self.border.append((self.cycles, value & 15))

    def call(self, address, limit=20000):
        self.s = 0xfd
        self.push(0xff)
        self.push(0xfe)
        self.pc = address
        start = self.cycles
        for _ in range(limit):
            if self.pc == 0xffff:
                return self.cycles-start
            self.step()
        raise AssertionError(f'kernel did not return at {self.pc:04x}')


class BootMachine(Cpu):
    """Start as BASIC SYS does: IRQs enabled, RAM not preinitialized by ELF."""
    def __init__(self, memory):
        super().__init__(memory, [(0, 65536)])
        self.p = U
        self.unsafe_low_writes = []

    def write(self, address, value):
        if 0x0200 <= address < 0x0800 and not self.p & I:
            self.unsafe_low_writes.append((address, self.pc))
        super().write(address, value)


def basic_sys_entry(memory, load_address):
    """Parse the tokenized BASIC program instead of assuming a SYS address."""
    line = load_address
    visited = set()
    while line not in visited and line + 4 < 65536:
        visited.add(line)
        following = int.from_bytes(memory[line:line+2], 'little')
        if following == 0:
            break
        assert line + 4 < following < 65536, 'invalid BASIC line link'
        body = bytes(memory[line+4:following])
        if 0x9e in body:
            operand = body[body.index(0x9e)+1:].split(b'\0', 1)[0].strip()
            assert operand.isdigit(), f'non-numeric BASIC SYS operand: {operand!r}'
            entry = int(operand)
            assert 0 <= entry <= 65535, 'BASIC SYS address out of range'
            return entry
        line = following
    raise AssertionError('PRG contains no tokenized BASIC SYS entry')


def test_prg_boot(expected, symbols, prg_path):
    """Exercise the shipped bytes and CRT relocation before entering main.

    Kernel tests use relocated ELF VMAs intentionally; boot tests MUST NOT do so.
    A missing .data copy otherwise disappears because the test already performed
    that copy on the runtime's behalf.
    """
    prg = prg_path.read_bytes()
    assert len(prg) >= 4, 'short PRG'
    load_address = int.from_bytes(prg[:2], 'little')
    assert load_address + len(prg) - 2 <= 65536, 'PRG load range exceeds RAM'
    memory = bytearray((address * 73 + 0xa5) & 255 for address in range(65536))
    memory[load_address:load_address+len(prg)-2] = prg[2:]
    memory[0] = 0x2f
    memory[1] = 0x37
    entry = basic_sys_entry(memory, load_address)
    assert entry == symbols['_start'], 'BASIC SYS does not address linked startup'
    machine = BootMachine(memory)
    machine.pc = entry
    for instructions in range(100000):
        if machine.pc == symbols['main']:
            break
        machine.step()
    else:
        raise AssertionError(f'raw PRG CRT never reached main: PC=${machine.pc:04x}')
    failures = []
    initialized_bytes = 0
    for name in ('data', 'zp_data'):
        start = symbols[f'__{name}_start']
        length = symbols[f'__{name}_size']
        actual = machine.mem[start:start+length]
        wanted = expected[start:start+length]
        initialized_bytes += length
        if actual != wanted:
            mismatch = next(i for i, (a, b) in enumerate(zip(actual, wanted)) if a != b)
            failures.append(f'.{name.replace("_", ".")} not initialized at ${start+mismatch:04x}: '
                            f'got ${actual[mismatch]:02x}, expected ${wanted[mismatch]:02x}')
    zeroed_bytes = 0
    for name in ('bss', 'zp_bss'):
        start = symbols[f'__{name}_start']
        length = symbols[f'__{name}_size']
        zeroed_bytes += length
        if any(machine.mem[start:start+length]):
            failures.append(f'.{name.replace("_", ".")} not zeroed')
    if machine.unsafe_low_writes:
        address, pc = machine.unsafe_low_writes[0]
        failures.append(f'IRQs enabled while overwriting low RAM ${address:04x} at PC=${pc:04x}')
    if not machine.p & I:
        failures.append('IRQs still enabled at main entry')
    assert not failures, 'raw PRG boot failed: ' + '; '.join(failures)
    report = {'passed': True, 'load_address': load_address, 'basic_sys_entry': entry,
              'main_entry': symbols['main'], 'instructions': instructions,
              'cpu_cycles': machine.cycles, 'initialized_bytes_verified': initialized_bytes,
              'zeroed_bytes_verified': zeroed_bytes, 'low_ram_writes_irq_safe': True,
              'prg_sha256': hashlib.sha256(prg).hexdigest()}
    print('Raw PRG boot: SYS', entry, '-> main;', initialized_bytes,
          'initialized bytes and', zeroed_bytes, 'zeroed bytes verified; early IRQ mask verified')
    return report


def test_boot_regression_guards(expected, symbols, prg_path):
    """Prove the raw-byte test rejects the two diagnosed startup defects.

    Mutations exist only in memory; the PRG and ELF artifacts are never changed.
    This complements the positive boot test when the original failing removable
    media is no longer connected to the host.
    """
    prg = prg_path.read_bytes()
    load = int.from_bytes(prg[:2], 'little')
    cases = [
        ('missing_data_copy', symbols['__do_copy_data'], bytes([0xea]*3), '.data not initialized'),
        ('missing_early_sei', symbols['_start'], bytes([0xea]), 'IRQs enabled while overwriting low RAM'),
    ]
    results = []
    for name, address, replacement, expected_error in cases:
        broken = bytearray(prg)
        offset = 2 + address - load
        assert 2 <= offset < len(prg), f'{name} test address outside PRG'
        if name == 'missing_data_copy':
            assert broken[offset] == 0x20, 'copy-data initializer is no longer a JSR'
        else:
            assert broken[offset] == 0x78, 'startup no longer begins with SEI'
        broken[offset:offset+len(replacement)] = replacement
        class MemoryPRG:
            def read_bytes(self):
                return bytes(broken)
        try:
            test_prg_boot(expected, symbols, MemoryPRG())
        except AssertionError as error:
            assert expected_error in str(error), f'{name} failed for an unrelated reason: {error}'
            results.append(name)
        else:
            raise AssertionError(f'boot test accepted regression: {name}')
    print('Boot negative controls rejected:', ', '.join(results))
    return results


def test_audio(memory, symbols):
    machine = Machine(memory.copy())
    m = machine.mem
    state = symbols['havoc_audio_chunk_state']
    pages = [0xc0, 0xc3, 0xc6, 0xc9, 0xe0, 0xe3]
    for i, page in enumerate(pages):
        for name, value in [('codes', page), ('pcm', page+1), ('lut', page+2)]:
            m[symbols[f'havoc_audio_{name}_hi']+i] = value
    m[0xf6] = 5
    cycles = set()
    cycles.add(machine.call(symbols['havoc_raster_work']))
    rng = random.Random(640032)
    predicted = [0, 0]
    checked = 0
    for iteration in range(240):
        slot = iteration % 6
        page = pages[slot] << 8
        book = [[rng.randrange(-32768, 32768) for _ in range(8)] for _ in range(2)]
        codes = [rng.randrange(64) for _ in range(128)]
        for f, code in enumerate(codes):
            m[page+2*f] = code
        for code in range(64):
            for channel in range(2):
                delta = book[channel][(code >> (channel*3)) & 7] & 65535
                m[page+512+channel*128+code] = delta & 255
                m[page+576+channel*128+code] = delta >> 8
        reset = iteration % 7 == 0
        m[symbols['havoc_audio_chunk_units']+slot] = 64
        m[symbols['havoc_audio_chunk_reset']+slot] = reset
        m[state+slot] = 1
        cycles.add(machine.call(symbols['havoc_raster_work']))
        assert m[state+slot] == 2
        if reset:
            predicted = [0, 0]
        for f in range(0, 128, 2):
            cycles.add(machine.call(symbols['havoc_raster_work']))
            for sub in range(2):
                code = codes[f+sub]
                for channel in range(2):
                    predicted[channel] = max(-32768, min(32767, predicted[channel] + book[channel][(code >> (channel*3)) & 7]))
                    expected = min(127, (predicted[channel]+128) >> 8) & 255
                    assert m[page+256+(f+sub)*2+channel] == expected, (iteration, f, channel)
                    checked += 1
        cycles.add(machine.call(symbols['havoc_raster_work']))
        assert m[state+slot] == 3
        m[state+slot] = 0
    print('Audio cycles:', sorted(cycles), 'samples checked:', checked)
    assert cycles == {256}, f'nonconstant audio callback: {cycles}'
    return checked


def test_raster(memory, symbols):
    machine = Machine(memory.copy())
    m = machine.mem
    for col in range(128):
        for y in range(256):
            m[0x4000+col*256+y] = ((2*col+y*3)&15) | (((2*col+1+y*3)&15)<<4)
    nibbles = symbols['havoc_raster_nibbles']
    for i in range(256):m[nibbles+i]=i>>4
    m[0xf6]=5
    tested=0
    schedules=json.loads((ROOT/'target-c64/raster-timing.json').read_text())['phase_schedules']
    for x in [0,1,2,55,56,127,128,199,200,254,255]:
        for y in [0,1,127,239,255]:
            m[symbols['havoc_raster_x']]=x
            machine.call(symbols['havoc_raster_prepare'])
            machine.y=y;machine.border=[]
            cycles=machine.call(symbols['havoc_raster_odd_line' if x&1 else 'havoc_raster_even_line'])
            assert len(machine.border)==201
            actual=[color for _,color in machine.border[:200]]
            expected=[((x+i)%256+y*3)&15 for i in range(200)]
            assert actual==expected,(x,y,next((i for i,(a,b) in enumerate(zip(actual,expected)) if a!=b),None))
            relative=[t-machine.border[0][0] for t,_ in machine.border]
            schedule=schedules[x&1]
            reference=[s['cycle']-schedule['stores'][0]['cycle'] for s in schedule['stores']]
            reference.append(schedule['black_store_cycle']-schedule['stores'][0]['cycle'])
            assert relative==reference,(x,y,relative[:12],reference[:12])
            tested+=1
    print('Raster phases/wrap cases:',tested,'dots per line:',400)
    return tested


class BeamMachine(Machine):
    """Instruction-clock NTSC beam for full-frame control-flow regression.

    Turbo is modeled as a constant 63 CPU cycles/PHI2, NTSC as 65 PHI2/line
    and 263 lines/frame. D031 index zero charges subsequent CPU instructions at
    one cycle/PHI2 until turbo resumes. This does not model FPGA latch phase,
    bus contention, video scaling, or physical timing; those require captures.
    """
    CPU_PER_PHI2 = 63
    PHI2_PER_LINE = 65
    LINES_PER_FRAME = 263
    UNITS_PER_LINE = CPU_PER_PHI2 * PHI2_PER_LINE

    def __init__(self, memory, initial_line=100):
        super().__init__(memory)
        self.beam_units = initial_line * self.UNITS_PER_LINE
        self.speed_factor = 1
        self.step_factor = 1
        self.step_start_cycles = 0
        self.inside_step = False
        self.beam_border = []

    def event_units(self):
        if self.inside_step:
            return self.beam_units + (self.cycles - self.step_start_cycles) * self.step_factor
        return self.beam_units

    def read(self, address):
        if address in (0xd011, 0xd012):
            line = (self.event_units() // self.UNITS_PER_LINE) % self.LINES_PER_FRAME
            if address == 0xd012:
                return line & 255
            return (self.mem[address] & 127) | (128 if line >= 256 else 0)
        return super().read(address)

    def write(self, address, value):
        if address == 0xd020:
            self.beam_border.append((self.event_units(), value & 15, self.y))
        elif address == 0xd031:
            assert value & 15 in (0, 15), 'beam test only models 1 MHz and maximum turbo'
            self.speed_factor = self.CPU_PER_PHI2 if value & 15 == 0 else 1
        super().write(address, value)

    def step(self):
        self.step_start_cycles = self.cycles
        self.step_factor = self.speed_factor
        self.inside_step = True
        super().step()
        self.beam_units += (self.cycles - self.step_start_cycles) * self.step_factor
        self.inside_step = False


def test_raster_frames(memory, symbols):
    """Catch forced double-frame waits when the 240-row window crosses line 263."""
    cases = []
    for top in (14, 31):
        for camera_x in (20, 21):
            machine = BeamMachine(memory.copy())
            m = machine.mem
            camera_y = 239  # Also exercise source-cache Y wrap independently.
            for column in range(128):
                for row in range(256):
                    m[0x4000+column*256+row] = ((column*2+row*3)&15) | (((column*2+1+row*3)&15)<<4)
            nibbles = symbols['havoc_raster_nibbles']
            for index in range(256):
                m[nibbles+index] = index >> 4
            m[0xf6] = 5  # Audio handoff takes its verified fixed-time empty path.
            m[symbols['havoc_raster_top']] = top
            m[symbols['havoc_raster_delay']] = 13
            m[symbols['havoc_raster_x']] = camera_x
            m[symbols['havoc_raster_y']] = camera_y
            starts = []
            for frame in range(2):
                machine.beam_border = []
                machine.call(symbols['havoc_raster_frame'], limit=1000000)
                trace = machine.beam_border
                assert len(trace) == 240*201, (top, camera_x, frame, 'wrong D020 write count', len(trace))
                first_line = trace[0][0] // machine.UNITS_PER_LINE
                starts.append(first_line)
                assert first_line % 263 == top, (top, camera_x, frame, 'wrong starting beam line', first_line % 263)
                for row in range(240):
                    writes = trace[row*201:(row+1)*201]
                    actual_line = writes[0][0] // machine.UNITS_PER_LINE
                    assert actual_line == first_line+row, (top, camera_x, frame, row, 'skipped or duplicate beam row', actual_line)
                    assert all(t // machine.UNITS_PER_LINE == actual_line for t, _, _ in writes), (top, camera_x, frame, row, 'pixels spill into next scanline')
                    source_y = (camera_y+row)&255
                    assert all(y == source_y for _, _, y in writes), (top, camera_x, frame, row, 'wrong source row')
                    actual = [color for _, color, _ in writes]
                    expected = [((camera_x+x)%256+source_y*3)&15 for x in range(200)] + [0]
                    assert actual == expected, (top, camera_x, frame, row, 'wrong frame pixel order')
            assert starts[1]-starts[0] == 263, (top, camera_x, 'consecutive frames are not one refresh apart', starts)
            cases.append({'top': top, 'camera_x': camera_x, 'frames': 2,
                          'rows_per_frame': 240, 'writes_per_frame': 240*201,
                          'start_spacing_lines': starts[1]-starts[0],
                          'last_line': (top+239)%263})
    print('Full-frame NTSC beam model:', len(cases), 'cases; 240 rows and consecutive refreshes verified, including 31..262,0..7')
    return {'model': '63 CPU/PHI2 turbo; 65 PHI2/line; 263 lines/frame; 1 MHz resync instructions',
            'hardware_measured': False, 'cases': cases}


class ReuMachine(Machine):
    def __init__(self,memory,blob):
        super().__init__(memory)
        self.reu=bytearray(0x1000000)
        self.reu[0x1000:0x1000+len(blob)]=blob
        self.output=bytearray()
    def read(self,address):
        if 0xdd04<=address<=0xdd07:
            return ((0xffffffff-self.cycles//63) >> ((address-0xdd04)*8))&255
        return super().read(address)
    def write(self,address,value):
        super().write(address,value)
        if address==0xdf01:
            m=self.mem
            ram=int.from_bytes(m[0xdf02:0xdf04],'little')
            reu=int.from_bytes(m[0xdf04:0xdf07],'little')
            count=int.from_bytes(m[0xdf07:0xdf09],'little') or 65536
            if value==0x91:m[ram:ram+count]=self.reu[reu:reu+count]
            elif value==0x90:
                block=m[ram:ram+count]
                self.reu[reu:reu+count]=block
                if 0xfe0000<=reu<0xff0000:self.output.extend(block)
            else:raise AssertionError(('unexpected DMA',value))


def test_real_clip(memory,symbols,fixture=None):
    fixture=fixture or ROOT/'target/music/diagnostic/hammond-rocker.dlt'
    blob=fixture.read_bytes()
    frames=int.from_bytes(blob[8:12],'little');books=int.from_bytes(blob[18:20],'little')
    stream=32+books*32
    groups=(frames+3)//4;short=(groups//books)*4;long=groups%books
    pcm=bytearray();predictors=[0,0]
    for frame in range(frames):
        boundary=long*(short+4)
        book=frame//(short+4) if frame<boundary else long+(frame-boundary)//short
        for channel in range(2):
            bit=(frame*2+channel)*3
            code=sum(((blob[stream+(bit+s)//8]>>((bit+s)%8))&1)<<s for s in range(3))
            delta=struct.unpack_from('<h',blob,32+book*32+channel*16+code*2)[0]
            predictors[channel]=max(-32768,min(32767,predictors[channel]+delta))
            pcm.append(min(127,(predictors[channel]+128)>>8)&255)
    machine=ReuMachine(memory.copy(),blob);m=machine.mem
    m[2:4]=bytes([0,0xd0]);m[0xf6]=5
    for i,page in enumerate([0xc0,0xc3,0xc6,0xc9,0xe0,0xe3]):
        for name,base in [('codes',page),('pcm',page+1),('lut',page+2)]:
            m[symbols[f'havoc_audio_{name}_hi']+i]=base
    needed=len(pcm)+1024
    for _ in range(2000):
        # Model playback consuming published output, so producer never fills its ring.
        m[symbols['consumed']:symbols['consumed']+4]=m[symbols['produced']:symbols['produced']+4]
        machine.call(symbols['service_audio'],limit=300000)
        assert not m[symbols['havoc_fault']]
        for _ in range(65):machine.call(symbols['havoc_raster_work'])
        if len(machine.output)>=needed:break
    assert len(machine.output)>=needed
    expected=(pcm+pcm)[:needed]
    actual=machine.output[:needed]
    assert actual==expected,next((i for i,(a,b) in enumerate(zip(actual,expected)) if a!=b),None)
    print('Linked C producer + assembly decoder + REU ring:',needed,'PCM bytes match independent DLT5 oracle, including loop')
    return needed


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf',type=Path,default=ROOT/'dist/prototype/havocmd-prototype.prg.elf')
    parser.add_argument('--prg',type=Path,default=ROOT/'dist/prototype/havocmd-prototype.prg')
    parser.add_argument('--fixture',type=Path,default=ROOT/'target/music/diagnostic/hammond-rocker.dlt')
    parser.add_argument('--report',type=Path,default=ROOT/'dist/prototype/kernel-tests.json')
    args=parser.parse_args()
    memory,symbols=image(args.elf)
    assert symbols['__data_end'] <= 0x800 and symbols['__bss_end'] <= 0x800
    assert symbols['__zp_bss_start']+symbols['__zp_bss_size'] < 0xf0
    boot=test_prg_boot(memory,symbols,args.prg)
    boot_guards=test_boot_regression_guards(memory,symbols,args.prg)
    checked=test_audio(memory,symbols)
    raster=test_raster(memory,symbols)
    frame_model=test_raster_frames(memory,symbols)
    clip=test_real_clip(memory,symbols,args.fixture)
    metadata=json.loads(args.fixture.with_suffix('.json').read_text())
    report={'machine_code_tests_passed':True,'audio_samples_checked':checked,
            'raster_cases':raster,'real_clip_pcm_bytes':clip,'hardware_measured':False,'passed':False,
            'audio_profile':metadata['profile'],'boot':boot,'boot_negative_controls':boot_guards,
            'frame_beam_model':frame_model,
            'prg_sha256':boot['prg_sha256']}
    args.report.write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
