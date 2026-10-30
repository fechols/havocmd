"""Execute the linked level renderer. No physical timing is claimed."""
from pathlib import Path
import importlib.util
import json
import os
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('prototype_tests',ROOT/'tools/test-prototype.py')
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)


def pixel(x,y):
    x &=255;y &=255
    return (x*7+(x>>4)*3+y*11+(y>>4)*5+((x*y)>>5))&15


def main():
    out=ROOT/'target/tilt-test';out.mkdir(parents=True,exist_ok=True)
    sdk=Path(os.environ.get('LLVM_MOS','/home/doomc64u/.local/llvm-mos'))
    subprocess.run([sys.executable,'-B',str(ROOT/'tools/generate_tilt.py')],check=True)
    prg=out/'tilt.prg'
    subprocess.run([str(sdk/'bin/mos-c64-clang'),'-Os','-g',str(ROOT/'target-c64/asm/tilt.S'),
        str(ROOT/'tools/tilt-test-stub.S'),'-Wl,--undefined=havoc_raster_frame',
        '-Wl,--undefined=havoc_raster_init','-o',str(prg)],check=True)
    memory,s=p.image(prg.with_suffix('.prg.elf'))
    assert not any(name in s for name in ('tilt_initial_x','tilt_remainder_lo','tilt_remainder_hi',
                                         'tilt_center_y','havoc_tilt_last_k','havoc_tilt_remainder'))
    machine=p.Machine(memory.copy());machine.call(s['havoc_raster_init'])
    assert machine.mem[s['havoc_tilt_k']]==0
    for c in range(128):
        for y in range(256):machine.mem[0x4000+c*256+y]=pixel(c*2,y)|(pixel(c*2+1,y)<<4)
    memory=machine.mem.copy()
    entries=[int.from_bytes(memory[s['havoc_tilt_entries']+i*2:s['havoc_tilt_entries']+i*2+2],'little') for i in range(128)]
    kernel_start=s['havoc_tilt_pixels'];kernel_end=kernel_start+3053
    kernel=bytes(memory[kernel_start:kernel_end])
    prepare_cycles=set();prepare_cases=0
    for x in range(256):
        for y in (0,1,63,64,127,128,239,255):
            machine.mem[s['havoc_raster_x']]=x;machine.mem[s['havoc_raster_y']]=y
            machine.mem[s['havoc_tilt_k']]=(x+y)&255
            prepare_cycles.add(machine.call(s['havoc_raster_prepare']))
            assert machine.mem[s['havoc_tilt_k']]==0
            assert machine.mem[s['havoc_tilt_row_x']]==x and machine.mem[s['havoc_tilt_row_y']]==y
            assert bytes(machine.mem[kernel_start:kernel_end])==kernel,'prepare modified fixed pixel kernel'
            prepare_cases+=1
    assert prepare_cycles=={36},prepare_cycles
    cases=0
    for c,entry in enumerate(entries):
        assert machine.mem[entry+9]==0xea,(c,'step slot must stay NOP')
        for y in (0,1,7,127,128,239,254,255):
            machine.mem[0xef]=101;machine.y=y;machine.border=[]
            machine.call(entry,limit=4000);trace=machine.border
            assert len(trace)==203,(c,y,len(trace))
            expected=[pixel(c*2+j,y) for j in range(202)]
            assert [v for _,v in trace]==expected+[0],(c,y,'pixels')
            for j in range(101):
                assert trace[j*2+1][0]-trace[j*2][0]==16,(c,j,'low-high')
                gap=16 if j==100 or (c+j+1)%4!=3 else 14
                assert trace[j*2+2][0]-trace[j*2+1][0]==gap,(c,j,'high-low')
            cases+=1
    setup_cycles=set()
    for x in range(256):
        machine.mem[s['havoc_raster_x']]=x;machine.mem[s['havoc_raster_y']]=83
        machine.call(s['havoc_raster_prepare'])
        setup_cycles.add(machine.call(s['havoc_tilt_row_setup']))
        assert machine.mem[s['havoc_tilt_row_x']]==x
        assert int.from_bytes(machine.mem[0xed:0xef],'little')==entries[x//2]
        machine.border=[];before=machine.cycles;machine.call(s['havoc_tilt_line'],limit=5000)
        visible=machine.border[x&1:(x&1)+200]
        assert [v for _,v in visible]==[pixel(x+j,83) for j in range(200)],(x,'visible pixels')
        assert visible[0][0]-before==557,(x,visible[0][0]-before)
        # Representative latch phase:200 displayed pixels occupy400 dots.
        times=[t-visible[0][0] for t,_ in visible]
        assert [int((t+4.5)*8//63) for t in times]==list(range(0,400,2)),x
    assert setup_cycles=={166},setup_cycles
    beam_cases=[]
    for x in (*range(8),254,255):
        beam=p.BeamMachine(memory.copy());m=beam.mem
        m[s['havoc_tilt_k']]=127;m[s['havoc_raster_x']]=x;m[s['havoc_raster_y']]=239
        starts=[]
        for frame in range(2):
            beam.beam_border=[];beam.call(s['havoc_raster_frame'],limit=1500000)
            trace=beam.beam_border;assert len(trace)==240*203,(x,len(trace))
            start=trace[0][0]//beam.UNITS_PER_LINE;starts.append(start)
            assert start%263==31,(x,start%263)
            for row in range(240):
                writes=trace[row*203:(row+1)*203]
                assert all(t//beam.UNITS_PER_LINE==start+row for t,_,_ in writes),(x,row,'beam')
                expected=[pixel((x&254)+j,239+row) for j in range(202)]
                assert [v for _,v,_ in writes]==expected+[0],(x,row,'geometry')
            assert m[s['havoc_tilt_k']]==0
            assert m[s['havoc_raster_late_lines']]==0,(x,'late',m[s['havoc_raster_late_lines']])
            assert bytes(m[kernel_start:kernel_end])==kernel,'frame modified fixed pixel kernel'
        assert starts[1]-starts[0]==263,(x,'refresh gap',starts)
        beam_cases.append({'k':0,'x':x,'frames':2,'rows':240})
    report={'hardware_measured':False,'static_level_renderer':True,'pixel_cases':cases,
        'row_setup_cases':256,'row_setup_cycles':166,'phase_cases':256,'beam_cases':beam_cases,
        'pixel_footprint':[202,240],'visible_pixel_footprint':[200,240],
        'k_range':[0,0],'angle_tables_bytes':0,'pixel_kernel_immutable':True,
        'prepare_cases':prepare_cases,'prepare_cycles':sorted(prepare_cycles),
        'maximum_prepare_phi2_ticks_at_63x':(max(prepare_cycles)+62)//63}
    (out/'tilt-tests.json').write_text(json.dumps(report,indent=2)+'\n')
    print('Level renderer:',cases,'pixel cases,256 phases,166-cycle row setup,20 full beam frames passed.',flush=True)


if __name__=='__main__':main()
