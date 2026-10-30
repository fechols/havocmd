"""Execute the actual timer assembly through byte/cascade/32-bit rollovers.

This models live CIA counter reads; an optional read-latch variant is exercised
as extra robustness coverage. B propagation delays0..3 PHI2 are modeled, not a
claim about the physical C64U implementation. No hardware is accessed.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('timer_oracle', ROOT/'tools/test-prototype.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class TimerMachine(oracle.Machine):
    def configure(self, value, divider, phase, cascade_delay, latched=False):
        self.cycles = 0
        self.initial, self.divider, self.phase = value, divider, phase
        self.cascade_delay, self.latched = cascade_delay, latched
        self.high_latches = [0x55, 0xaa]
        self.low_samples = []
        self.writes = []
        self.io_writes = []

    def read(self, address):
        if 0xdd04 <= address <= 0xdd07:
            ticks = (self.cycles+self.phase)//self.divider
            logical = (self.initial-ticks) & 0xffffffff
            high_half = address >= 0xdd06
            physical = ((self.initial-ticks+self.cascade_delay) & 0xffffffff
                        if high_half else logical)
            if address == 0xdd04:
                self.low_samples.append(logical)
            index = int(high_half)
            shift = 16*index
            if self.latched:
                if not address & 1:
                    self.high_latches[index] = (physical >> (shift+8)) & 255
                else:
                    return self.high_latches[index]
            return (physical >> ((address-0xdd04)*8)) & 255
        return super().read(address)

    def write(self, address, value):
        if 0xd000 <= address <= 0xdfff:
            self.io_writes.append((address, value))
        super().write(address, value)


def main():
    sdk = Path(os.environ.get('LLVM_MOS', str(Path.home()/'.local/llvm-mos')))
    with tempfile.TemporaryDirectory(prefix='havoc-timer-') as directory:
        directory = Path(directory)
        fixture = directory/'fixture.c'
        fixture.write_text('#include <stdint.h>\nextern uint32_t timer_read(void);\nint main(void){return (int)timer_read();}\n')
        prg = directory/'timer.prg'
        subprocess.run([str(sdk/'bin/mos-c64-clang'), '-Oz', str(fixture),
                        str(ROOT/'target-c64/asm/timer.S'), '-o', str(prg)], check=True)
        memory, symbols = oracle.image(Path(str(prg)+'.elf'))
        machine = TimerMachine(memory)
        counts, cycles = 0, set()
        # Each possible CPU/PHI2 alignment and every rollover position within
        # a complete first attempt, including carry into each higher byte.
        for boundary in (0x100, 0x10000, 0x1000000, 0x100000000):
            for offset in range(-8, 96):
                for divider in (1, 63):
                    for phase in range(divider):
                        for delay in range(4):
                            machine.configure((boundary+offset) & 0xffffffff, divider, phase, delay)
                            cycles.add(machine.call(symbols['timer_read'], limit=2000))
                            result = (machine.a | machine.x << 8 |
                                      machine.mem[symbols['__rc2']] << 16 |
                                      machine.mem[symbols['__rc3']] << 24)
                            assert result == machine.low_samples[-1], (boundary, offset, divider, phase, delay, hex(result))
                            assert not machine.io_writes
                            counts += 1
        # A stale read-latch model must also converge; it cannot accidentally
        # depend on fresh, undocumented latches left behind by another caller.
        for value in (0, 1, 0xff, 0x100, 0xffff, 0x10000, 0xffffff, 0x1000000, 0xffffffff):
            for phase in range(63):
                for delay in range(4):
                    machine.configure(value, 63, phase, delay, latched=True)
                    cycles.add(machine.call(symbols['timer_read'], limit=2000))
                    result = (machine.a | machine.x << 8 | machine.mem[symbols['__rc2']] << 16 |
                              machine.mem[symbols['__rc3']] << 24)
                    assert result == machine.low_samples[-1], (value, phase, delay, 'latch')
                    assert not machine.io_writes
                    counts += 1
        machine.configure(0x81234567, 63, 0, 0)
        normal = machine.call(symbols['timer_read'])
        report = {'hardware_measured': False, 'cases': counts, 'normal_cpu_cycles': normal,
                  'observed_cpu_cycle_range': [min(cycles), max(cycles)],
                  'old_linked_c_cpu_cycles': 288, 'cascade_delay_phi2_tested': [0, 1, 2, 3],
                  'live_reads_and_stale_latch_variant': True, 'writes_timer_registers': False}
        path = ROOT/'target-c64/build/timer-tests.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report, indent=2)+'\n')
        print(f'Timer assembly passed {counts} rollover cases; normal {normal} CPU cycles; '
              f'observed range {min(cycles)}..{max(cycles)}; no I/O writes.', flush=True)


if __name__ == '__main__':
    main()
