"""Execute the actual foreground6502 helpers against independent byte oracles."""
from pathlib import Path
import importlib.util
import json
import os
import random
import subprocess

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('prototype_tests',ROOT/'tools/test-prototype.py')
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)

def main():
    out=ROOT/'target/audio-fast-test';out.mkdir(parents=True,exist_ok=True)
    sdk=Path(os.environ.get('LLVM_MOS','/home/doomc64u/.local/llvm-mos'))
    prg=out/'audio-fast.prg'
    subprocess.run([str(sdk/'bin/mos-c64-clang'),'-Os','-g',str(ROOT/'target-c64/asm/audio_fast.S'),
        str(ROOT/'tools/audio-fast-test-stub.S'),'-Wl,--undefined=audio_unpack_fast',
        '-Wl,--undefined=audio_lut_fast','-Wl,--undefined=audio_timer16','-o',str(prg)],check=True)
    memory,s=p.image(prg.with_suffix('.prg.elf'));machine=p.Machine(memory)
    rng=random.Random(0x6510);unpack_cycles={};lut_cycles=set();cases=0
    for page in [0xc0,0xc3,0xc6,0xc9,0xe0,0xe3]:
        for n in range(3,97,3):
            for iteration in range(4):
                source=bytes(rng.randrange(256) for _ in range(n))
                machine.mem[0xe600:0xe600+n]=source
                machine.mem[page*256:(page+1)*256]=bytes([0xa5])*256
                expected=bytearray([0xa5])*256
                for group in range(n//3):
                    word=int.from_bytes(source[group*3:group*3+3],'little')
                    for sample in range(4):expected[group*8+sample*2]=(word>>(sample*6))&63
                machine.a=page;machine.x=n
                cycles=machine.call(s['audio_unpack_fast'],limit=6000)
                assert machine.mem[page*256:(page+1)*256]==expected,(page,n,iteration)
                unpack_cycles.setdefault(n,set()).add(cycles);cases+=1
    for page in [0xc2,0xc5,0xc8,0xcb,0xe2,0xe5]:
        for iteration in range(64):
            book=bytes(rng.randrange(256) for _ in range(32))
            machine.mem[0xe680:0xe6a0]=book
            machine.mem[page*256:(page+1)*256]=bytes([0xa5])*256
            expected=bytearray(256)
            for code in range(64):
                left=(code&7)*2;right=16+(code>>3)*2
                expected[code]=book[left];expected[code+64]=book[left+1]
                expected[code+128]=book[right];expected[code+192]=book[right+1]
            machine.a=page;lut_cycles.add(machine.call(s['audio_lut_fast'],limit=5000))
            assert machine.mem[page*256:(page+1)*256]==expected,(page,iteration)
    for value in (0,1,255,256,32767,32768,65535):
        machine.mem[0xdd04]=value&255;machine.mem[0xdd05]=value>>8
        assert machine.call(s['audio_timer16'])==20
        assert machine.a|(machine.x<<8)==value
    class Rollover(p.Machine):
        values=iter([1,0,0,0,254,0])
        def read(self,address):
            if address in (0xdd04,0xdd05):return next(self.values)
            return super().read(address)
    rollover=Rollover(memory.copy())
    assert rollover.call(s['audio_timer16'])==35
    assert rollover.a==254 and rollover.x==0
    report={'hardware_measured':False,'unpack_cases':cases,'timer16_cycles':20,'timer16_rollover_cycles':35,'lut_cases':384,
        'unpack_full_chunk_cycles':sorted(unpack_cycles[96]),'lut_cycles':sorted(lut_cycles),
        'unpack_cycles_by_packed_bytes':{k:sorted(v) for k,v in unpack_cycles.items()}}
    (out/'audio-fast-tests.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k!='unpack_cycles_by_packed_bytes'},indent=2))

if __name__=='__main__':main()
