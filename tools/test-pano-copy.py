"""Execute the real panorama copy helpers and validate bytes, ABI and timing."""
from pathlib import Path
import importlib.util
import json
import os
import subprocess
from native_cpu import OPCODES, OPERAND_BYTES, I

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('prototype_tests',ROOT/'tools/test-prototype.py')
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)


def instructions(memory,start):
    pc=start;result=[]
    while pc<start+256:
        name,mode,_=OPCODES[memory[pc]]
        size=1+OPERAND_BYTES[mode];result.append((pc,name,mode))
        pc+=size
        if name=='RTS':return result,pc-start
    raise AssertionError('missing helper RTS')


def main():
    out=ROOT/'target/pano-copy-test';out.mkdir(parents=True,exist_ok=True)
    sdk=Path(os.environ.get('LLVM_MOS','/home/doomc64u/.local/llvm-mos'))
    abi=out/'pano-copy-abi.c'
    abi.write_text('''#include <stdint.h>
extern void pano_copy_band(uint8_t *destination,const uint8_t *source,uint16_t count);
extern void pano_copy_row(uint8_t *destination,const uint8_t *source,uint16_t count);
extern void pano_copy_strip(uint8_t *destination,const uint8_t *source,uint16_t count);
void band_abi(void) { pano_copy_band((uint8_t *)0xeb7f,(const uint8_t *)0xe6c0,64); }
void row_abi(void) { pano_copy_row((uint8_t *)0x40ff,(const uint8_t *)0xee80,128); }
void strip_abi(void) { pano_copy_strip((uint8_t *)0xbfff,(const uint8_t *)0xe6c3,5); }
''')
    prg=out/'pano-copy.prg'
    subprocess.run([str(sdk/'bin/mos-c64-clang'),'-Os','-g',str(ROOT/'target-c64/asm/pano_copy.S'),
        str(ROOT/'tools/audio-fast-test-stub.S'),str(abi),'-o',str(prg),
        *[f'-Wl,--undefined={name}' for name in ('pano_copy_band','pano_copy_row','pano_copy_strip','band_abi','row_abi','strip_abi')]],check=True)
    image,symbols=p.image(prg.with_suffix('.prg.elf'))
    band_instructions,band_bytes=instructions(image,symbols['pano_copy_band'])
    row_instructions,row_bytes=instructions(image,symbols['pano_copy_row'])
    strip_instructions,strip_bytes=instructions(image,symbols['pano_copy_strip'])
    band_patches=[pc+2 for pc,name,mode in band_instructions if (name,mode)==('STA','aby')]
    row_patches=[pc+2 for pc,name,mode in row_instructions if (name,mode)==('STA','abx')]
    strip_sources=[pc+1 for pc,name,mode in strip_instructions if (name,mode)==('LDA','abx')]
    strip_stores=[pc+2 for pc,name,mode in strip_instructions if (name,mode)==('STA','aby')]
    strip_count=[pc+1 for pc,name,mode in strip_instructions if (name,mode)==('CPX','imm')]
    strip_patches=strip_sources+strip_stores+strip_count
    assert len(band_patches)==8 and len(row_patches)==1
    assert len(strip_sources)==len(strip_stores)==8 and len(strip_count)==1
    assert symbols['pano_copy_band']%128==0 and symbols['pano_copy_row']%32==0
    assert all(symbols[name]<0x2000 for name in ('pano_copy_band','pano_copy_row','pano_copy_strip','band_abi','row_abi','strip_abi'))
    template=image.copy()
    template[0x2000:]=bytes((address*73+address//17+0x5a)&255 for address in range(0x2000,65536))
    for address in range(2,34):template[address]=(address*71+23)&255
    template[2:4]=b'\x00\xd0'
    def check(memory,expected,patches,destinations,name,count,write_count=None):
        for address in patches:memory[address]=0x11
        machine=p.Machine(memory);machine.a=count&255;machine.x=count>>8
        cycles=machine.call(symbols[name],limit=10000)
        assert machine.s==0xfd and machine.p&I,(name,'stack or IRQ state')
        assert machine.mem[:0x1fc]==expected[:0x1fc],(name,'ZP/register/low-memory corruption')
        assert machine.mem[0x1fe:]==expected[0x1fe:],(name,'destination or guard mismatch')
        allowed=set(patches)|destinations
        assert all(address in allowed for address in machine.writes),(name,'out-of-contract write')
        assert sum(address in destinations for address in machine.writes)==(count if write_count is None else write_count),(name,'write count')
        return cycles
    band_cycles=set();band_cases=0
    for base in (0xe700,0xeb00):
        for packed in range(128):
            memory=template.copy();memory[4:6]=(base+packed).to_bytes(2,'little');memory[6:8]=b'\xc0\xe6'
            memory[0xe6c0:0xe700]=bytes((i*37+packed*11+(i>>3)*17)&255 for i in range(64))
            expected=memory.copy();destinations=set()
            for c in range(8):
                for r in range(8):
                    at=base+r*128+((packed+c)&127);destinations.add(at)
                    expected[at]=memory[0xe6c0+c*8+r]
            for r,address in enumerate(band_patches):expected[address]=(base>>8)+(r>>1)
            band_cycles.add(check(memory,expected,band_patches,destinations,'pano_copy_band',64));band_cases+=1
    row_cycles=set();row_cases=0
    for source in range(0xe700,0xef00,128):
        for row in range(256):
            memory=template.copy();memory[4:6]=(0x4000+row).to_bytes(2,'little');memory[6:8]=source.to_bytes(2,'little')
            memory[source:source+128]=bytes((i*53+row*17+source//128)&255 for i in range(128))
            expected=memory.copy();destinations={0x4000+c*256+row for c in range(128)}
            for c in range(128):expected[0x4000+c*256+row]=memory[source+c]
            expected[row_patches[0]]=0xc0
            row_cycles.add(check(memory,expected,row_patches,destinations,'pano_copy_row',128));row_cases+=1
    strip_cycles=set();strip_cases=0;strip_full_cycles=set()
    def strip_case(first,count,column,row):
        nonlocal strip_cases
        memory=template.copy();source=0xe6c0+first;destination=0x4000+column*256+row
        memory[4:6]=destination.to_bytes(2,'little');memory[6:8]=source.to_bytes(2,'little')
        memory[0xe6c0:0xe700]=bytes((i*37+first*19+count*11+row)&255 for i in range(64))
        expected=memory.copy();destinations=set()
        for c in range(8):
            expected[strip_sources[c]]=(source+c*8)&255
            expected[strip_stores[c]]=0x40+((column+c)&127)
            for r in range(count):
                at=0x4000+((column+c)&127)*256+((row+r)&255)
                destinations.add(at);expected[at]=memory[source+c*8+r]
        expected[strip_count[0]]=count
        cycles=check(memory,expected,strip_patches,destinations,'pano_copy_strip',count,count*8)
        strip_cycles.add(cycles)
        if count==8:strip_full_cycles.add(cycles)
        strip_cases+=1
    # Every valid first/end pair and physicalY phase, at all seven cache-page
    # wrapping positions plus representative low/mid/nonwrapping columns.
    for first in range(8):
        for count in range(1,9-first):
            for column in (0,63,64,*range(120,128)):
                for row in range(256):strip_case(first,count,column,row)
        print(f'Strip source first{first}: {strip_cases} cases passed',flush=True)
    # Also exercise every cache page for full/one-byte clips at row boundaries.
    for column in range(128):
        for row in (0,249,255):
            strip_case(0,8,column,row);strip_case(7,1,column,row)
    for name,destination,source,count in (('band_abi',0xeb7f,0xe6c0,64),('row_abi',0x40ff,0xee80,128),('strip_abi',0xbfff,0xe6c3,5)):
        machine=p.Machine(template.copy());before=machine.mem.copy();machine.call(symbols[name])
        if count==64:
            for c in range(8):
                for r in range(8):assert machine.mem[0xeb00+r*128+((127+c)&127)]==before[source+c*8+r]
        elif count==128:
            for c in range(128):assert machine.mem[0x4000+c*256+255]==before[source+c]
        else:
            for c in range(8):
                for r in range(count):assert machine.mem[0x4000+((127+c)&127)*256+((255+r)&255)]==before[source+c*8+r]
        source_end=source+count if count>=64 else 0xe700
        assert machine.mem[source:source_end]==before[source:source_end]
        assert machine.s==0xfd and machine.mem[2:4]==b'\x00\xd0'
        assert machine.mem[4:6]==destination.to_bytes(2,'little') and machine.mem[6:8]==source.to_bytes(2,'little')
    assert band_cycles=={797},band_cycles
    assert row_cycles=={2705},row_cycles
    report={'passed':True,'hardware_measured':False,'band_cases':band_cases,'row_cases':row_cases,'strip_cases':strip_cases,
        'compiled_c_abi_cases':3,'band_cycles':sorted(band_cycles),'row_cycles':sorted(row_cycles),
        'strip_cycles':sorted(strip_cycles),'strip_full_cycles':sorted(strip_full_cycles),
        'strip_max_phi2_at_63x':round(max(strip_cycles)/63,3),
        'band_max_phi2_at_63x':round(max(band_cycles)/63,3),'row_max_phi2_at_63x':round(max(row_cycles)/63,3),
        'band_code_bytes':band_bytes,'row_code_bytes':row_bytes,'strip_code_bytes':strip_bytes,'software_stack_bytes':0,
        'static_scratch_bytes':0,'callee_preserved_registers_checked':True,'source_unchanged':True,
        'destination_guards_checked':True,'all_imaginary_registers_preserved':True,
        'band_column_wrap_checked':True,'last_row_store_page':0xbf,'row_patch_after_last_store':0xc0,
        'strip_column_and_row_wrap_checked':True,'strip_every_valid_first_end_checked':True,
        'fixed_contracts':{'band':{'count':64,'source':0xe6c0,'destination_bases':[0xe700,0xeb00]},
                           'row':{'count':128,'destination_base':0x4000,'source_alignment':128},
                           'strip':{'count_range':[1,8],'source_range':[0xe6c0,0xe6c7],
                                    'source_first_plus_count_max':8,'columns':8,'source_stride':8,
                                    'destination_pages':[0x40,0xbf],'physical_rows':256}}}
    (out/'pano-copy-tests.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


if __name__=='__main__':main()
