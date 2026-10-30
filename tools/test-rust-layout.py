"""Audit the linked Rust/6502 image, including retired-startup references.

This checks ELF layout and static relocation/control-flow properties. It does
not replace the raw-PRG boot model or establish physical hardware timing.
"""
import argparse
import copy
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import struct

from native_cpu import OPCODES, OPERAND_BYTES
from build_fingerprint import BANK_GATE_NAMES

ROOT = Path(__file__).resolve().parents[1]
BOOT_SECTIONS = {'.disk_init', '.bootmath'}
BANK_GATES = {name: name+'_banked' for name in BANK_GATE_NAMES}


def require(condition, message):
    if not condition:
        raise AssertionError(message)


@dataclass
class Section:
    name: str
    kind: int
    flags: int
    address: int
    offset: int
    size: int
    link: int
    info: int
    alignment: int
    entry_size: int


@dataclass
class Symbol:
    name: str
    address: int
    size: int
    info: int
    section: int


class Elf:
    def __init__(self, path):
        self.path = Path(path)
        self.data = self.path.read_bytes()
        require(self.data[:6] == b'\x7fELF\x01\x01', 'expected little-endian ELF32')
        offset = struct.unpack_from('<I', self.data, 32)[0]
        size, count, names_index = struct.unpack_from('<HHH', self.data, 46)
        raw = [struct.unpack_from('<10I', self.data, offset+i*size) for i in range(count)]
        names = self.data[raw[names_index][4]:raw[names_index][4]+raw[names_index][5]]
        string = lambda table, start: table[start:table.index(0, start)].decode()
        self.sections = [Section(string(names, row[0]), *row[1:]) for row in raw]
        self.memory = bytearray(65536)
        self.tables = {}
        self.symbols = []
        for index, section in enumerate(self.sections):
            if section.flags & 2 and section.kind == 1:
                require(section.address+section.size <= 65536, f'{section.name}: out-of-range image')
                self.memory[section.address:section.address+section.size] = self.data[section.offset:section.offset+section.size]
            if section.kind == 2:
                strings = self.sections[section.link]
                table = self.data[strings.offset:strings.offset+strings.size]
                symbols = []
                for at in range(section.offset, section.offset+section.size, section.entry_size):
                    name, value, length, info, _, sid = struct.unpack_from('<IIIBBH', self.data, at)
                    symbols.append(Symbol(string(table, name), value, length, info, sid))
                self.tables[index] = symbols
                self.symbols.extend(symbols)
        self.names = {s.name: s for s in self.symbols if s.name}
        self.functions = [s for s in self.symbols if s.info & 15 == 2 and s.size]

    def symbol(self, name):
        require(name in self.names, f'missing stable ELF symbol: {name}')
        return self.names[name]

    def owner(self, address):
        matches = [s for s in self.functions if s.address <= address < s.address+s.size]
        return min(matches, key=lambda s: s.size) if matches else None

    def relocations(self):
        result = []
        for section in self.sections:
            if section.kind != 4:  # SHT_RELA
                continue
            source = self.sections[section.info]
            if not source.flags & 2:
                continue
            for at in range(section.offset, section.offset+section.size, section.entry_size):
                offset, info, addend = struct.unpack_from('<IIi', self.data, at)
                target = self.tables[section.link][info >> 8]
                address = offset if source.address <= offset < source.address+source.size else source.address+offset
                result.append((source, address, target, target.address+addend, info & 255))
        return result

    def overlay_references(self):
        return [r for r in self.relocations() if r[2].section < len(self.sections) and
                self.sections[r[2].section].name in BOOT_SECTIONS]


def walk_function(elf, function, entry=None):
    """Follow both branch arms, direct jumps and call fallthroughs inside one function."""
    start, end = function.address, function.address+function.size
    pending, instructions = [start if entry is None else entry], {}
    while pending:
        address = pending.pop()
        if address in instructions:
            continue
        require(start <= address < end, f'{function.name}: CFG leaves its extent at ${address:04x}')
        opcode = elf.memory[address]
        require(opcode in OPCODES, f'{function.name}: unsupported opcode ${opcode:02x} at ${address:04x}')
        name, mode, _ = OPCODES[opcode]
        size = 1+OPERAND_BYTES[mode]
        require(address+size <= end, f'{function.name}: incomplete instruction')
        operand = int.from_bytes(elf.memory[address+1:address+size], 'little')
        instructions[address] = (name, mode, operand, size)
        if name in ('RTS', 'RTI', 'BRK'):
            continue
        if name == 'JSR':
            callee = elf.owner(operand)
            if callee and ('4fail17' in callee.name or 'panic' in callee.name):
                # Rust's fail/panic signatures return !; following this call
                # as if it returned would invent edges between basic blocks.
                continue
        if name == 'JMP':
            require(mode == 'abs', f'{function.name}: indirect CFG jump requires manual review')
            if start <= operand < end:
                pending.append(operand)
            # External direct JMP is a tail call; no fallthrough.
            continue
        if mode == 'rel':
            displacement = operand if operand < 128 else operand-256
            pending.append(address+size+displacement)
        pending.append(address+size)
    return instructions


def layout(elf):
    allocated = [s for s in elf.sections if s.flags & 2 and s.size]
    ranges = {'.basic_header':(0x801,0x4000), '.earlycode':(0x801,0x900),
              '.text':(0x801,0x4000), '.rodata':(0x801,0x4000),
              '.data':(0x200,0x800), '.bss':(0x200,0x800), '.noinit':(0x200,0x800),
              '.tailcode':(0x200,0x800), '.highcode':(0xef00,0xfffa),
              '.midcode':(0xcc00,0xcf80), '.disk_init':(0x7000,0xc000),
              '.bootmath':(0x7000,0xc000), '.bankcode':(0xd000,0xe000), '.zp':(0x22,0x90),
              '.zp.data':(0x22,0x90), '.zp.bss':(0x22,0x90)}
    for section in allocated:
        require(section.name in ranges, f'unreviewed allocated section: {section.name}')
        low, high = ranges[section.name]
        require(low <= section.address <= section.address+section.size <= high,
                f'{section.name} exceeds reserved ${low:04x}..${high:04x}')
    for index, a in enumerate(allocated):
        for b in allocated[index+1:]:
            require(a.address+a.size <= b.address or b.address+b.size <= a.address,
                    f'allocated sections overlap: {a.name}/{b.name}')
    require(elf.symbol('__stack').address == 0xd000, 'wrong software stack top')
    require(elf.symbol('__heap_start').address <= 0x800, 'low RAM exceeds BASIC header')
    for name in ('high', 'mid', 'tail', 'bank'):
        start = elf.symbol(f'__{name}_start').address
        end = elf.symbol(f'__{name}_end').address
        load = elf.symbol(f'__{name}_load').address
        require(0x4000 <= load <= load+end-start <= 0x7000, f'{name}: initializers exceed startup copy area')
    require(elf.symbol('__bank_start').address == 0xd000 and
            0xd000 < elf.symbol('__bank_end').address <= 0xe000, 'bank code must fit RAM beneath I/O')
    require('rotation_state' not in elf.names and 'motion_propose' not in elf.names,
            'rotation state or motion code remains in the scrolling-only player')
    angle = elf.symbol('havoc_tilt_k')
    require(angle.size in (0,1) and elf.sections[angle.section].name == '.data' and
            elf.memory[angle.address] == 0, 'level renderer debug angle must initialize to zero in data')
    require(elf.symbol('audio_decode_track').size == 19, 'AudioTrack ABI must be19 bytes')
    slots = [s for s in elf.symbols if 'musicdisk' in s.name and '5SLOTS' in s.name]
    require(len(slots) == 1 and slots[0].size == 60, 'two packed DiskBundles must occupy60 bytes')
    for name in ('havoc_audio_chunk_state', 'havoc_audio_chunk_units', 'havoc_audio_chunk_reset',
                 'havoc_audio_codes_hi', 'havoc_audio_pcm_hi', 'havoc_audio_lut_hi'):
        require(elf.symbol(name).size == 6, f'{name}: assembly ABI expects six slots')
    for name in ('disk_error','disk_busy','uci_error','uci_busy','uci_complete','uci_packet_ready'):
        require(elf.symbol(name).size == 1, f'{name}: instrumentation ABI must remain one byte')
    require(elf.symbol('disk_chunks').size == 2, 'disk_chunks ABI must remain two bytes')
    dma = elf.symbol('reu_execute')
    require(dma.size == 4 and elf.memory[dma.address:dma.address+4] == b'\x8d\x01\xdf\x60',
            'opaque REU execution leaf must be STA $DF01; RTS')
    return {s.name: {'address':s.address, 'bytes':s.size} for s in allocated}



def bank_safety(elf):
    """Audit exact opaque gates and every statically reachable pure callee.

    Indirect cache pointers are additionally checked by native cache tests and
    the raw-PRG model, which gives RAM-under-I/O its own backing storage.
    """
    gates = {elf.symbol(name).address: name for name in BANK_GATES}
    roots, approved_operands = [], {}
    for name, body_name in BANK_GATES.items():
        gate, body = elf.symbol(name), elf.symbol(body_name)
        require(elf.sections[body.section].name == '.bankcode', f'{body_name}: body is not banked')
        require(not 0xd000 <= gate.address < 0xe000, f'{name}: gate cannot live under I/O')
        expected = b'\x48\xa9\x34\x85\x01\x68\x20'+body.address.to_bytes(2,'little')+b'\x48\xa9\x35\x85\x01\x68\x60'
        require(gate.size == len(expected) and elf.memory[gate.address:gate.address+gate.size] == expected,
                f'{name}: opaque bank gate must preserve A and restore$35')
        approved_operands[gate.address+7] = body.address
        roots.append(body)
    relocation_names = set()
    for source, address, target, destination, kind in elf.relocations():
        caller = elf.owner(address)
        if target.name in ('__bank_start', '__bank_end', '__bank_load'):
            require(caller is not None and caller.name == 'boot_relocate',
                    'bank initialization metadata referenced outside boot_relocate')
            relocation_names.add(target.name)
            continue
        if target.section >= len(elf.sections) or elf.sections[target.section].name != '.bankcode':
            continue
        if source.name == '.bankcode':
            continue
        require(address in approved_operands and approved_operands[address] == destination,
                'resident reference enters bank code without an opaque gate')
    require({'__bank_start','__bank_load'} <= relocation_names,
            'boot_relocate must copy the banked code image')
    pending, visited, instruction_count = roots[:], {}, 0
    while pending:
        function = pending.pop()
        if function.address in visited:
            continue
        require(function.address not in gates, 'pure banked helper calls a nested bank gate')
        require(elf.sections[function.section].name not in BOOT_SECTIONS,
                'pure banked helper reaches retired startup')
        visited[function.address] = function.name
        for pc, (name, mode, operand, size) in walk_function(elf, function).items():
            instruction_count += 1
            require(name not in ('CLI','PLP','RTI','BRK'),
                    f'pure banked helper can enable interrupts or escape its call: {name}')
            require(mode not in ('zpx','zpy'), 'pure helper has an unbounded zero-page indexed access')
            if mode in ('abx','aby'):
                require(all(not (0xd000 <= ((operand+i)&65535) < 0xe000 or
                                  ((operand+i)&65535) in (0,1)) for i in range(256)),
                        f'pure banked helper indexed MMIO/CPU banking access: {function.name}')
            if name in ('JSR','JMP'):
                require(mode == 'abs', 'pure helper has an indirect control transfer')
                if function.address <= operand < function.address+function.size:
                    continue
                require(operand not in gates, 'pure banked helper calls a nested bank gate')
                callee = elf.owner(operand)
                require(callee is not None and callee.address == operand,
                        f'pure helper has an unowned/interior callee at${operand:04x}')
                pending.append(callee)
            elif mode in ('abs','abx','aby'):
                require(not 0xd000 <= operand < 0xe000,
                        f'pure banked helper accesses MMIO: {function.name} at${pc:04x}')
            if mode in ('zp','zpx','zpy','abs','abx','aby'):
                require(operand not in (0,1), 'pure banked helper changes or reads CPU banking')
    return {'passed':True, 'gate_count':len(gates), 'banked_bytes':elf.symbol('__bank_end').address-0xd000,
            'pure_functions':sorted(visited.values()), 'instructions_checked':instruction_count,
            'startup_copy_references':sorted(relocation_names),
            'nested_gates_rejected':True, 'direct_mmio_rejected':True}


def overlays(elf):
    main = elf.symbol('main')
    pano = elf.symbol('pano_init')
    calls = walk_function(elf, main)
    barriers = [pc for pc, (name, _, operand, _) in calls.items() if name == 'JSR' and operand == pano.address]
    require(len(barriers) == 1, 'expected one main→pano_init startup boundary')
    hot = walk_function(elf, main, barriers[0]+3)
    # These four linker-owned CRT stubs execute by straight-line fallthrough
    # before main. Check exact stub addresses/targets, not a section whitelist.
    crt_begin = elf.symbol('havoc_boot_begin').address
    call_main = elf.symbol('__after_main').address-3
    require(elf.memory[call_main] == 0x20 and
            int.from_bytes(elf.memory[call_main+1:call_main+3],'little') == main.address,
            'CRT main call does not match the expected initialization boundary')
    crt_calls = {}
    for stem in ('copy_data','copy_zp_data','zero_bss','zero_zp_bss'):
        stub = elf.symbol('__do_'+stem).address
        target = elf.symbol('__'+stem)
        require(crt_begin <= stub < call_main and elf.memory[stub] == 0x20 and
                int.from_bytes(elf.memory[stub+1:stub+3],'little') == target.address,
                f'CRT startup stub/target mismatch: {stem}')
        crt_calls[stub+1] = ('__do_'+stem,target)
    pc = crt_begin
    while pc < call_main:
        require(elf.memory[pc] in OPCODES, 'unsupported opcode in CRT initialization prefix')
        name, mode, _ = OPCODES[elf.memory[pc]]
        require(mode != 'rel' and name not in ('JMP','RTS','RTI','BRK'),
                'CRT initialization is no longer a straight-line startup prefix')
        if name == 'JSR': require(pc+1 in crt_calls, 'unreviewed CRT startup call')
        pc += 1+OPERAND_BYTES[mode]
    require(pc == call_main, 'CRT instruction boundary misses the call to main')
    for source, address, target, destination, kind in elf.relocations():
        if crt_begin <= destination < call_main:
            require(source.name == '.basic_header' or crt_begin <= address < call_main,
                    f'resident reference reenters retired CRT startup: ${address:04x}')
    pano_calls = walk_function(elf, pano)
    init_calls = [(pc, operand) for pc, (name, _, operand, _) in pano_calls.items()
                  if name == 'JSR' and 0x7000 <= operand < 0xc000]
    require(len(init_calls) == 1, 'pano_init must have exactly one startup initialization call')
    init = elf.owner(init_calls[0][1])
    require(init and 'panorama' in init.name and 'initialize' in init.name,
            'pano_init startup bridge points at an unexpected function')
    dma = elf.symbol('reu_fetch').address
    after_init = walk_function(elf, pano, init_calls[0][0]+3)
    require(any(name in ('JSR','JMP') and operand == dma for name, _, operand, _ in after_init.values()),
            'pano_init must return from initialize before the seed DMA')
    # No direct path may execute DMA before initialization has returned.
    before = [pano.address]
    visited = set()
    while before:
        pc = before.pop()
        if pc in visited or pc == init_calls[0][0]:
            continue
        visited.add(pc)
        name, mode, operand, size = pano_calls[pc]
        require(not (name in ('JSR','JMP') and operand == dma), 'seed DMA precedes startup return')
        if name in ('RTS','RTI','BRK'):
            continue
        if name == 'JMP':
            if operand in pano_calls: before.append(operand)
            continue
        before.append(pc+size)
        if mode == 'rel': before.append(pc+size+(operand if operand<128 else operand-256))
    approved, boot_internal = [], 0
    for source, address, target, destination, kind in elf.overlay_references():
        if source.name in BOOT_SECTIONS:
            boot_internal += 1
            continue
        instruction = address-1
        caller = elf.owner(address)
        if address in crt_calls:
            stub_name, expected = crt_calls[address]
            require(destination == expected.address, f'CRT target changed: {stub_name}')
            approved.append({'caller':stub_name,'instruction':instruction,'target':expected.name,
                             'destination':destination,'relocation_kind':kind})
            continue
        require(caller is not None, f'unowned resident reference to startup: ${address:04x}→{target.name}')
        require(elf.memory[instruction] in (0x20,0x4c),
                f'resident data/address reference to retired startup: {caller.name}→{target.name}')
        require(int.from_bytes(elf.memory[address:address+2], 'little') == destination,
                f'startup relocation does not match a direct16-bit control-transfer operand: ${address:04x}')
        if caller.name == 'main':
            require(instruction in calls and instruction not in hot,
                    f'main hot path reaches retired startup: {target.name}')
        elif caller.name == 'pano_init':
            require(instruction == init_calls[0][0] and destination == init.address,
                    f'unexpected pano_init startup reference: {target.name}')
        else:
            raise AssertionError(f'resident function reaches retired startup: {caller.name}→{target.name}')
        resolved = elf.owner(destination)
        approved.append({'caller':caller.name, 'instruction':instruction,
                         'target':target.name or (resolved.name if resolved else ''),
                         'destination':destination, 'relocation_kind':kind})
    require(approved and boot_internal, 'missing ELF relocation coverage; link with --emit-relocs')
    return {'startup_only_references':approved, 'internal_boot_references':boot_internal,
            'main_post_seed_instructions_checked':len(hot), 'seed_dma_after_initialize':True,
            'crt_startup_only_calls_checked':len(crt_calls),
            'rotation_sections_absent':True}


def negative_checks(original):
    """Mutate only in-memory audit inputs; never write the production ELF."""
    def rejected(action, message):
        try:
            action()
        except AssertionError as error:
            require(message in str(error), f'unexpected negative-check failure: {error}')
        else:
            raise AssertionError(f'audit accepted injected fault: {message}')

    bad_gate = copy.deepcopy(original)
    gate = bad_gate.symbol('pano_picture')
    bad_gate.memory[gate.address+11] = 0x37
    rejected(lambda: bank_safety(bad_gate), 'opaque bank gate')
    for opcode, target, message in ((0xad,0xd020,'accesses MMIO'),
                                    (0x20,original.symbol('controller_adjacent').address,'nested bank gate')):
        bad_bank = copy.deepcopy(original)
        body = bad_bank.symbol('pano_picture_banked')
        bad_bank.memory[body.address:body.address+4] = bytes([opcode,target&255,target>>8,0x60])
        rejected(lambda: bank_safety(bad_bank), message)
    for code, message in ((bytes([0xa2,0x80,0xbd,0x80,0xcf,0x60]),'indexed MMIO/CPU banking'),
                          (bytes([0xbd,0x80,0xff,0x60]),'indexed MMIO/CPU banking'),
                          (bytes([0xb5,0x80,0x60]),'unbounded zero-page indexed'),
                          *[(bytes([opcode,0x60]),'enable interrupts or escape')
                            for opcode in (0x58,0x28,0x40,0x00)]):
        bad_bank = copy.deepcopy(original)
        body = bad_bank.symbol('pano_picture_banked')
        bad_bank.memory[body.address:body.address+len(code)] = code
        rejected(lambda: bank_safety(bad_bank), message)
    wrong_abi = copy.deepcopy(original)
    wrong_abi.symbol('audio_decode_track').size = 20
    rejected(lambda: layout(wrong_abi), 'AudioTrack ABI')
    for caller_name, message in (('uci_start','resident function reaches retired startup'),
                                  ('main','main hot path reaches retired startup')):
        elf = copy.deepcopy(original)
        caller = elf.symbol(caller_name)
        entry = None
        if caller_name == 'main':
            pano_address = elf.symbol('pano_init').address
            entry = next(pc+3 for pc, instruction in walk_function(elf, caller).items()
                         if instruction[0] == 'JSR' and instruction[2] == pano_address)
        pc = next(pc for pc, instruction in walk_function(elf, caller, entry).items()
                  if instruction[0] == 'JSR' and
                  not (elf.owner(instruction[2]) and
                       ('4fail17' in elf.owner(instruction[2]).name or
                        'panic' in elf.owner(instruction[2]).name)))
        target = elf.symbol('boot_prepare')
        elf.memory[pc+1:pc+3] = target.address.to_bytes(2,'little')
        references = elf.overlay_references()
        references.append((elf.sections[caller.section],pc+1,target,target.address,3))
        elf.overlay_references = lambda: references
        rejected(lambda: overlays(elf), message)
    elf = copy.deepcopy(original)
    caller = elf.symbol('uci_start')
    pc = next(pc for pc, instruction in walk_function(elf, caller).items() if instruction[0] == 'JSR')
    target = elf.symbol('__do_copy_data')
    elf.memory[pc+1:pc+3] = target.address.to_bytes(2,'little')
    references = elf.relocations()
    references.append((elf.sections[caller.section],pc+1,target,target.address,3))
    elf.relocations = lambda: references
    rejected(lambda: overlays(elf), 'resident reference reenters retired CRT startup')
    return ['wrong bank restoration', 'banked MMIO access', 'nested bank gate', 'indexed access reaches MMIO',
            'indexed address wraps CPU port', 'unbounded zero-page indexed access',
            'banked CLI', 'banked PLP', 'banked RTI', 'banked BRK', 'wrong packed AudioTrack size', 'resident helper calls retired startup',
            'main playback loop calls retired startup', 'resident helper reenters CRT startup']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf',type=Path,default=ROOT/'dist/havocmd/havocmd.prg.elf')
    parser.add_argument('--report',type=Path,default=ROOT/'target-c64/build/rust-layout-tests.json')
    args=parser.parse_args()
    elf=Elf(args.elf)
    report={'rust_layout_passed':True, 'hardware_measured':False,
            'elf_sha256':hashlib.sha256(elf.data).hexdigest(),
            'sections':layout(elf), 'overlay_audit':overlays(elf), 'bank_audit':bank_safety(elf),
            'negative_checks':negative_checks(elf)}
    args.report.parent.mkdir(parents=True,exist_ok=True)
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS Rust ELF: memory regions, packed ABI, stable globals and retired-startup relocation/CFG audit.')


if __name__=='__main__':
    main()
