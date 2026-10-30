"""Check the actual LLVM-MOS memcpy helper's bytes, ABI and write boundaries."""
from pathlib import Path
import importlib.util
import json
import os
import random
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('prototype_tests', ROOT/'tools/test-prototype.py')
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


def main():
    out = ROOT/'target/memory-test'
    out.mkdir(parents=True, exist_ok=True)
    sdk = Path(os.environ.get('LLVM_MOS', '/home/doomc64u/.local/llvm-mos'))
    prg = out/'memory.prg'
    abi = out/'memory-abi.c'
    abi.write_text('''#include <stdint.h>
extern void reu_execute(uint8_t command, uint8_t *ram);
void reu_abi_fetch(void) { reu_execute(0x91, (uint8_t *)0xe600); }
void reu_abi_stash(void) { reu_execute(0x90, (uint8_t *)0xe6c0); }
''')
    subprocess.run([str(sdk/'bin/mos-c64-clang'), '-Os', '-g',
        str(ROOT/'target-c64/asm/memory.S'), str(ROOT/'tools/audio-fast-test-stub.S'), str(abi),
        '-Wl,--undefined=memcpy', '-Wl,--undefined=reu_abi_fetch',
        '-Wl,--undefined=reu_abi_stash', '-o', str(prg)], check=True)
    image, symbols = p.image(prg.with_suffix('.prg.elf'))
    rng = random.Random(0x640016)
    cases = [(0x2000+a, 0x8000+((a*79) & 255), n)
             for a in range(256) for n in (0, 1, 2, 19, 255, 256, 257, 511, 512)]
    cases += [(0x1000, 0x9000, n) for n in (1023, 1024, 4095, 4096, 16383, 24576, 28672)]
    cases += [(0x2000+rng.randrange(256), 0x9000+rng.randrange(256), rng.randrange(8192))
              for _ in range(128)]
    maximum_cycles = 0
    for source, destination, count in cases:
        memory = image.copy()
        # Protect the return pointer, callee-preserved imaginary registers,
        # unrelated RAM, source bytes and both destination guard bytes.
        for address in range(2, 34):
            memory[address] = (address*71+23) & 255
        memory[0x1000:] = bytes((address*73+address//17+count) & 255 for address in range(0x1000, 65536))
        memory[4:6] = destination.to_bytes(2, 'little')
        memory[6:8] = source.to_bytes(2, 'little')
        expected = memory.copy()
        expected[destination:destination+count] = memory[source:source+count]
        machine = p.Machine(memory)
        machine.a, machine.x = count & 255, count >> 8
        cycles = machine.call(symbols['memcpy'], limit=max(100, count*9+100))
        maximum_cycles = max(maximum_cycles, cycles)
        assert machine.mem[4:6] == destination.to_bytes(2, 'little'), (source, destination, count, 'return pointer')
        assert machine.s == 0xfd, 'hardware stack is unbalanced'
        for start, end in [(0, 6), (11, 0x1fc), (0x1fe, 65536)]:
            assert machine.mem[start:end] == expected[start:end], (source, destination, count, start, end)
        assert all(6 <= at <= 10 or destination <= at < destination+count for at in machine.writes), (
            source, destination, count, 'write outside ABI scratch and destination')
    leaf_cycles = set()
    for command in range(256):
        machine = p.Machine(image.copy())
        machine.a = command
        machine.mem[4:6] = bytes([0xc0, 0xe6])
        leaf_cycles.add(machine.call(symbols['reu_execute']))
        assert machine.mem[0xdf01] == command and machine.writes == [0xdf01]
        assert machine.mem[4:6] == bytes([0xc0, 0xe6]) and machine.s == 0xfd
    for name, command, pointer in [('reu_abi_fetch', 0x91, 0xe600), ('reu_abi_stash', 0x90, 0xe6c0)]:
        machine = p.Machine(image.copy())
        machine.call(symbols[name])
        assert machine.mem[0xdf01] == command
        assert int.from_bytes(machine.mem[4:6], 'little') == pointer, 'RAM pointer ABI mismatch'
        assert machine.writes.count(0xdf01) == 1 and machine.s == 0xfd
    assert leaf_cycles == {10}, leaf_cycles
    report = {'hardware_measured': False, 'cases': len(cases),
              'maximum_count': max(n for _, _, n in cases), 'maximum_cycles': maximum_cycles,
              'software_stack_bytes': 0, 'static_scratch_bytes': 0,
              'abi_return_pointer_checked': True, 'callee_preserved_registers_checked': True,
              'destination_guards_checked': True, 'reu_command_cases': 256,
              'reu_compiled_c_abi_cases': 2, 'reu_command_leaf_cycles': sorted(leaf_cycles)}
    (out/'memory-tests.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
