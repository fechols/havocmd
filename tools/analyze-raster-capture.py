"""Compare physical UDP raster output to the exact indexed panorama cache."""
from pathlib import Path
import argparse,json
import numpy as np

ROOT=Path(__file__).resolve().parents[1]

def main():
    p=argparse.ArgumentParser();p.add_argument('capture',type=Path);a=p.parse_args()
    meta=json.loads((a.capture/'capture.json').read_text())
    frames=np.frombuffer((a.capture/'frames.idx').read_bytes(),np.uint8).reshape(-1,meta['height'],meta['width'])
    packed=np.frombuffer((ROOT/'target/art/lowland-approach.cache.bin').read_bytes(),np.uint8).reshape(128,256)
    source=np.empty((256,256),np.uint8);source[:,::2]=packed.T&15;source[:,1::2]=packed.T>>4
    ys=np.arange(8,190,13);xs=np.arange(8,376,11);yy,xx=np.meshgrid(ys,xs,indexing='ij');yy=yy.ravel();xx=xx.ravel()
    report=[]
    for index in [0,len(frames)//2,len(frames)-1]:
        frame=frames[index];observed=frame[yy,xx];best=(-1,None,None)
        for dy in range(256):
            sample=source[(yy+dy)[None,:]%256,((xx[None,:]+np.arange(512)[:,None])//2)%256]
            scores=np.mean(sample==observed,axis=1);dx=int(np.argmax(scores))
            if scores[dx]>best[0]:best=(float(scores[dx]),dx,dy)
        score,base_x,base_y=best
        row_shifts=[];row_scores=[]
        shifts=np.arange(-8,9)
        for y in range(meta['height']):
            x=np.arange(384)
            candidates=source[(y+base_y)%256,((x[None,:]+base_x+shifts[:,None])//2)%256]
            scores=np.mean(candidates==frame[y],axis=1);at=int(np.argmax(scores))
            row_shifts.append(int(shifts[at]));row_scores.append(float(scores[at]))
        item={'frame':index,'coarse_match':score,'physical_x_base':base_x,'source_y_base':base_y,
              'row_shift_counts':{str(s):row_shifts[:200].count(int(s)) for s in shifts},
              'row_shifts':row_shifts,'row_scores':row_scores,
              'black_rows':np.where(np.all(frame==0,axis=1))[0].tolist()}
        report.append(item)
        print(json.dumps({k:v for k,v in item.items() if k not in ('row_shifts','row_scores')}))
    (a.capture/'raster-analysis.json').write_text(json.dumps(report,indent=2)+'\n')
    # R3's measured alignment is source X+4 in the cropped384-dot stream.
    # Check EVERY pixel of EVERY frame against all allowed camera positions.
    positions=np.array([(x,y) for x in range(57) for y in range(17)])
    probes=source[positions[:,1,None]+yy,positions[:,0,None]+4+xx//2]
    details=[]
    for index,frame in enumerate(frames):
        scores=np.mean(probes==frame[yy,xx],axis=1)
        x,y=positions[int(np.argmax(scores))]
        expected=source[y+np.arange(meta['height'])[:,None],x+4+np.arange(384)[None,:]//2]
        errors=np.sum(expected!=frame,axis=1)
        details.append({'frame':index,'x':int(x),'y':int(y),
                        'mismatch_pixels':int(errors.sum()),
                        'error_rows':{str(i):int(n) for i,n in enumerate(errors) if n},
                        'fully_black':bool(np.all(frame==0))})
    summary={'frames':len(details),'perfect_frames':sum(d['mismatch_pixels']==0 for d in details),
             'fully_black_frames':sum(d['fully_black'] for d in details),'details':details}
    (a.capture/'all-frames-comparison.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps({k:v for k,v in summary.items() if k!='details'}))

if __name__=='__main__':main()
