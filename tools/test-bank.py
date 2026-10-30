"""Execute the real bank gates with distinct D000 RAM and I/O backing stores."""
from pathlib import Path
import importlib.util
import json
import os
import subprocess
from build_fingerprint import BANK_GATE_NAMES

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('prototype_tests',ROOT/'tools/test-prototype.py')
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)
GATES=BANK_GATE_NAMES
CAPTURE_REGISTERS=10


class BankMachine(p.Machine):
    def __init__(self,memory):
        super().__init__(memory)
        self.mem[0]=0x2f;self.mem[1]=0x35
        self.io=bytearray([0xa5])*4096
        self.port_writes=[];self.io_reads=0;self.io_writes=0;self.bank_fetches=0
        self.minimum_stack=0xfd

    def io_visible(self):
        pins=(self.mem[1]&self.mem[0])|((~self.mem[0])&255)
        return bool(pins&3) and bool(pins&4)

    def read(self,address):
        address&=65535
        if 0xd000<=address<0xe000 and self.io_visible():
            self.io_reads+=1
            return self.io[address-0xd000]
        return super().read(address)

    def write(self,address,value):
        address&=65535
        if address==1:self.port_writes.append(value)
        if 0xd000<=address<0xe000 and self.io_visible():
            self.io_writes+=1
            self.io[address-0xd000]=value&255
            return
        super().write(address,value)

    def step(self):
        assert self.p&p.I,'gate execution enabled interrupts'
        if 0xd000<=self.pc<0xe000:
            assert not self.io_visible(),'banked instruction fetch with I/O mapped'
            self.bank_fetches+=1
        super().step()
        self.minimum_stack=min(self.minimum_stack,self.s)
        assert self.p&p.I,'gate execution enabled interrupts'


def main():
    out=ROOT/'target/bank-test';out.mkdir(parents=True,exist_ok=True)
    sdk=Path(os.environ.get('LLVM_MOS','/home/doomc64u/.local/llvm-mos'))
    fixture=out/'bank-bodies.S'
    lines=['.section .bankcode,"ax",@progbits']
    for index,name in enumerate(GATES):
        base=0x3000+index*64
        lines += [f'.globl {name}_banked',f'{name}_banked:',
                  f'    sta ${base:04x}',f'    stx ${base+1:04x}',f'    sty ${base+2:04x}']
        for n in range(CAPTURE_REGISTERS):lines += [f'    lda ${4+n:02x}',f'    sta ${base+3+n:04x}']
        # A/X/Y and a pointer-sized rc2/3 result deliberately differ from inputs.
        # The source is RAM under I/O, not the separate poisoned I/O store.
        lines += ['    lda $dff3','    sta $04','    lda $dff4','    sta $05',
                  '    lda $dff0','    ldx $dff1','    ldy $dff2','    rts']
    fixture.write_text('\n'.join(lines)+'\n')
    linker=out/'bank.ld'
    linker.write_text('\n'.join(f'__rc{n} = {n+2};' for n in range(32))+'''\nMEMORY {
 ram (rwx) : ORIGIN = 0x0801, LENGTH = 0x37ff
 bank (rwx) : ORIGIN = 0xd000, LENGTH = 0x1000
}
SECTIONS {
 .text : { *(.text .text.* .highcode .highcode.*) } >ram
 .bankcode : { *(.bankcode .bankcode.*) } >bank
}
OUTPUT_FORMAT { SHORT(0x0801) FULL(ram) }
''')
    abi=out/'bank-abi.c'
    abi.write_text('''#include <stdint.h>
extern void pano_picture(void *picture,uint32_t address,uint16_t width,uint16_t height);
void picture_abi(void) { pano_picture((void *)0x1234,0x89abcdefUL,0x4567,0x3210); }
''')
    prg=out/'bank.prg'
    command=[str(sdk/'bin/mos-c64-clang'),'-Os','-g',str(ROOT/'target-c64/asm/bank.S'),
             str(fixture),str(abi),str(ROOT/'tools/audio-fast-test-stub.S'),
             '-nostdlib','-nostartfiles','-Wl,-e,main','-T',str(linker),'-o',str(prg)]
    command += [f'-Wl,--undefined={name}' for name in GATES]
    command += ['-Wl,--undefined=picture_abi']
    subprocess.run(command,check=True)
    image,symbols=p.image(prg.with_suffix('.prg.elf'))
    for name in GATES:
        assert not 0xd000<=symbols[name]<0xe000
        assert 0xd000<=symbols[name+'_banked']<0xdff0
    machine=BankMachine(image.copy())
    assert machine.read(0xd000)==0xa5
    machine.mem[1]=0x34;assert machine.read(0xd000)==image[0xd000]
    machine.mem[1]=0x35;machine.io_reads=0
    cases=0;cycles=set()
    for index,name in enumerate(GATES):
        base=0x3000+index*64
        for a in range(256):
            for x in range(256):
                y=(a+x*17)&255
                registers=bytes((a+x+n*37)&255 for n in range(30))
                result=bytes([a^0x5a,x^a,(a+x)&255,x^0xc3,a^0x96])
                machine.mem[2:4]=b'\x80\xcf'
                machine.mem[4:34]=registers
                machine.mem[0xdff0:0xdff5]=result
                machine.a=a;machine.x=x;machine.y=y
                machine.port_writes.clear();machine.writes.clear()
                cycles.add(machine.call(symbols[name],limit=200))
                assert machine.mem[base:base+3+CAPTURE_REGISTERS]==bytes([a,x,y])+registers[:CAPTURE_REGISTERS],(name,a,x,'arguments')
                assert (machine.a,machine.x,machine.y)==tuple(result[:3]),(name,a,x,'A/X/Y result')
                assert machine.mem[4:6]==result[3:5],(name,a,x,'pointer result')
                assert machine.mem[6:34]==registers[2:],(name,a,x,'remaining imaginary registers')
                assert machine.mem[2:4]==b'\x80\xcf','software stack pointer changed'
                assert machine.port_writes==[0x34,0x35],(name,a,x,machine.port_writes)
                assert machine.mem[1]==0x35 and machine.s==0xfd
                assert not machine.io_reads and not machine.io_writes
                assert all(at in (1,4,5) or base<=at<base+3+CAPTURE_REGISTERS for at in machine.writes)
                cases+=1
        print(f'Bank gate {name}: all65,536 A/X input pairs passed',flush=True)
    machine.port_writes.clear();machine.writes.clear()
    machine.call(symbols['picture_abi'],limit=200)
    base=0x3000+GATES.index('pano_picture')*64
    captured=machine.mem[base:base+3+CAPTURE_REGISTERS]
    assert captured[:2]==bytes([0xef,0xcd]),('32-bit argument low word',list(captured))
    assert captured[3:11]==bytes([0x34,0x12,0xab,0x89,0x67,0x45,0x10,0x32]),('pointer/u32/u16/u16 arguments',list(captured))
    assert machine.port_writes==[0x34,0x35] and machine.s==0xfd
    assert machine.minimum_stack==0xf9
    report={'hardware_measured':False,'gate_count':len(GATES),'argument_pairs_per_gate':65536,
            'cases':cases,'port_sequence':[0x34,0x35],'separate_io_reads':machine.io_reads,
            'separate_io_writes':machine.io_writes,'banked_instruction_fetches':machine.bank_fetches,
            'hardware_stack_min':machine.minimum_stack,'software_stack_changed':False,
            'argument_and_return_registers_checked':True,'interrupts_disabled_throughout':True,
            'compiled_picture_abi_cases':1,'picture_arguments':['pointer16','address32','width16','height16'],
            'fixture_call_cycles':sorted(cycles),
            'memory_map_primary_source':'https://www.devili.iki.fi/Computers/Commodore/C64/Programmers_Reference/Chapter_5/page_265.html'}
    (out/'bank-tests.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


if __name__=='__main__':main()
