"""Boot the delivered PRG with modeled peripherals, not pre-relocated ELF RAM.

Sampler completion is accelerated; this checks program flow and initialization,
not real FPGA timing or DMA throughput.
"""
import hashlib
import json
from pathlib import Path
from native_cpu import U

# test-prototype is also an executable script; import its existing CPU/DMA model.
import importlib.util
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('prototype_tests', ROOT/'tools/test-prototype.py')
prototype = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prototype)


class BootMachine(prototype.ReuMachine):
    def __init__(self, prg, reu, symbols, sampler=True):
        memory = bytearray([0xa5])*65536
        address = int.from_bytes(prg[:2], 'little')
        memory[address:address+len(prg)-2] = prg[2:]
        super().__init__(memory, b'')
        self.reu[:] = reu
        self.pc = symbols['_start']
        self.p = U  # BASIC's SYS normally enters with IRQs enabled.
        self.extra_ticks = 0
        self.measuring = False
        self.completed = False
        self.sampler = sampler
        self.pixel_writes = 0
        self.started = False

    def read(self, address):
        ticks = self.cycles//63+self.extra_ticks
        if 0xdd04 <= address <= 0xdd07:
            return ((0xffffffff-ticks) >> ((address-0xdd04)*8)) & 255
        if address == 0xd011:
            return (self.mem[address] & 0x7f) | (0x80 if ticks//65 % 263 >= 256 else 0)
        if address == 0xd012:
            return (ticks//65 % 263) & 255
        if address == 0xdf21:
            return 0x10 if self.sampler else 0xff
        if address == 0xdf20:
            if self.measuring and not self.completed:
                length = int.from_bytes(self.mem[0xdf29:0xdf2c], 'big')
                divider = int.from_bytes(self.mem[0xdf2e:0xdf30], 'big')
                self.extra_ticks += round(length/2*1022727/32000*divider/195)
                self.completed = True
                return 0
            return int(self.completed)
        if address in (0xdc00, 0xdc01):
            return 255
        return super().read(address)

    def write(self, address, value):
        super().write(address, value)
        if address == 0xdf20:
            self.measuring = value == 0x45
            self.completed = False
        if address == 0xdf40 and value == 0x43:
            self.started = True
        if address == 0xd020:
            self.pixel_writes += 1


def boot(prg, reu, symbols, fault=0, sampler=True):
    machine = BootMachine(prg, reu, symbols, sampler)
    for count in range(40_000_000):
        machine.step()
        if fault:
            expected_color = {1: 2, 2: 4, 3: 8}[fault]
            if machine.mem[symbols['havoc_fault']] == fault and machine.mem[0xd020] == expected_color:
                assert not machine.started
                print(f'Raw PRG boot: fault {fault} reaches visible color {expected_color}; {count+1} instructions')
                return
        elif machine.started and machine.mem[symbols['running']] == 1:
            assert machine.mem[symbols['havoc_fault']] == 0
            assert machine.mem[symbols['havoc_raster_frames']] > 0
            assert machine.pixel_writes > 200*240
            produced = int.from_bytes(machine.mem[symbols['produced']:symbols['produced']+4], 'little')
            assert produced >= 24576
            assert len(set(machine.reu[0xfe0000:0xfe0000+produced*2])) > 100
            print(f'Raw PRG boot: raster and stereo playback start; {produced} frames primed; {count+1} instructions')
            return
    produced = int.from_bytes(machine.mem[symbols['produced']:symbols['produced']+4], 'little')
    raise AssertionError(f'boot did not reach expected state; pc=${machine.pc:04x}, fault={machine.mem[symbols["havoc_fault"]]}, border={machine.mem[0xd020]}, produced={produced}')


def main():
    directory = ROOT/'dist/prototype'
    prg = (directory/'havocmd-prototype.prg').read_bytes()
    _, symbols = prototype.image(directory/'havocmd-prototype.prg.elf')
    reu = (directory/'havocmd-prototype.reu').read_bytes()
    boot(prg, bytes(len(reu)), symbols, fault=1)
    boot(prg, reu, symbols, fault=2, sampler=False)
    boot(prg, reu, symbols)
    (directory/'startup-tests.json').write_text(json.dumps({
        'raw_prg_boot_passed': True,
        'missing_reu_visible': True,
        'missing_sampler_visible': True,
        'peripherals': 'modeled; sampler completion accelerated',
        'prg_sha256': hashlib.sha256(prg).hexdigest(),
        'hardware_measured': False,
    }, indent=2)+'\n')


if __name__ == '__main__':
    main()
