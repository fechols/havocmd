"""Capture live C64U UDP video/audio without resetting or pausing the machine.

Protocol: https://1541u-documentation.readthedocs.io/en/latest/data_streams.html
Video is cropped by the firmware to 384 columns; it cannot prove HDMI edge coverage.
"""
import argparse
import array
import json
import math
from pathlib import Path
import select
import socket
import struct
import time
import urllib.request
import urllib.parse
import wave
import zlib

PALETTE = bytes.fromhex('000000 f7f7f7 8d2f34 6ad4cd 9835a4 4cb442 2c29b1 efef5d 984e20 5b3800 d1676d 4a4a4a 7b7b7b 9fef93 6d6aef b2b2b2')


def png(path, pixels, width, height):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload))+kind+payload+struct.pack('>I', zlib.crc32(kind+payload)&0xffffffff)
    rgb = bytearray()
    for y in range(height):
        rgb.append(0)
        for color in pixels[y*width:(y+1)*width]:
            rgb.extend(PALETTE[color*3:color*3+3])
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', width,height,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(rgb))+chunk(b'IEND',b''))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--host', default='192.168.66.178')
    parser.add_argument('--seconds', type=float, default=5)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def request(path, method='GET'):
        with opener.open(urllib.request.Request('http://'+args.host+path, method=method),timeout=8) as response:
            return response.read()
    info = json.loads(request('/v1/info'))
    settings = json.loads(request('/v1/configs/U64%20Specific%20Settings'))
    height = 240 if settings['U64 Specific Settings']['System Mode']=='NTSC' else 272
    route = socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
    route.connect((args.host,80))
    local = route.getsockname()[0]
    route.close()
    sockets = {}
    for name in ('video','audio'):
        stream = socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
        stream.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,4*1024*1024)
        stream.bind((local,0))
        stream.setblocking(False)
        sockets[name] = stream
    active=[]; frames=[]; audio=bytearray(); packets={'video':0,'audio':0}; lost={'video':0,'audio':0}; last={}; shapes={}
    current=None; pixels=None; rows=set(); frame_numbers=[]
    try:
        for name, stream in sockets.items():
            destination=local+':'+str(stream.getsockname()[1])
            response=json.loads(request('/v1/streams/'+name+':start?ip='+urllib.parse.quote(destination),'PUT'))
            if response.get('errors'):raise RuntimeError(response)
            active.append(name)
        start=time.monotonic()
        print('Capturing live C64U to',args.output,'receiver',local,flush=True)
        while time.monotonic()-start < args.seconds:
            ready,_,_=select.select(list(sockets.values()),[],[],0.2)
            for stream in ready:
                name=next(n for n,s in sockets.items() if s is stream)
                try: data,source=stream.recvfrom(65536)
                except BlockingIOError:continue
                if source[0]!=args.host:continue
                shape=(name,len(data))
                if shape not in shapes:
                    shapes[shape]=data[:24].hex()
                    print('Packet',name,len(data),data[:24].hex(),flush=True)
                if len(data)<2:continue
                sequence=struct.unpack_from('<H',data)[0]
                if name in last:
                    gap=(sequence-last[name]-1)&65535
                    if gap<32768:lost[name]+=gap
                last[name]=sequence;packets[name]+=1
                if name=='audio':
                    if (len(data)-2)%4:raise RuntimeError(('audio packet length',len(data)))
                    audio.extend(data[2:]);continue
                if len(data)<12:continue
                sequence,frame,line,width,count,bits,encoding=struct.unpack_from('<HHHHBBH',data)
                if width!=384 or bits!=4 or encoding!=0:raise RuntimeError(('unknown video format',width,bits,encoding))
                at=line&32767
                if frame!=current:
                    current=frame;pixels=bytearray(width*height);rows=set()
                for row in range(count):
                    y=at+row
                    if y>=height:continue
                    packed=data[12+row*width//2:12+(row+1)*width//2]
                    if len(packed)!=width//2:continue
                    pixels[y*width:(y+1)*width]=bytes(c for b in packed for c in (b&15,b>>4))
                    rows.add(y)
                if line&32768 and len(rows)==height:
                    frames.append(bytes(pixels));frame_numbers.append(frame)
    finally:
        for name in active:
            try:request('/v1/streams/'+name+':stop','PUT')
            except Exception as error:print('Stream stop failed:',name,error)
        for stream in sockets.values():stream.close()
    if not frames:raise RuntimeError(f'No complete video frames: packets={packets}, lost={lost}')
    (args.output/'frames.idx').write_bytes(b''.join(frames))
    for index in sorted(set([0,len(frames)//2,len(frames)-1])):
        png(args.output/f'frame-{index:04d}.png',frames[index],384,height)
    with wave.open(str(args.output/'audio.wav'),'wb') as out:
        out.setnchannels(2);out.setsampwidth(2);out.setframerate(47940 if height==240 else 47983);out.writeframes(audio)
    samples=array.array('h',audio)
    stats=[]
    for channel in range(2):
        values=samples[channel::2]
        stats.append({'peak':max(map(abs,values),default=0),'rms':math.sqrt(sum(v*v for v in values)/max(1,len(values)))})
    metadata={'device':info,'settings':settings,'seconds':args.seconds,'width':384,'height':height,'frames':len(frames),
              'frame_numbers':frame_numbers,'packets':packets,'missing_packets':lost,'audio_channels':stats,
              'video_source':'physical C64U UDP stream; outer HDMI columns omitted','audio_source':'physical C64U mixer UDP stream'}
    (args.output/'capture.json').write_text(json.dumps(metadata,indent=2)+'\n')
    print(json.dumps({k:metadata[k] for k in ('frames','packets','missing_packets','audio_channels')}),flush=True)


if __name__=='__main__':main()
