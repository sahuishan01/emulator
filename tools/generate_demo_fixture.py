#!/usr/bin/env python3
"""Original 64x64 homebrew command page and integer-to-vertex coordinate table."""
import argparse
from pathlib import Path
import struct
from generate_draw_fixtures import build, ROOT

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--check',action='store_true');a=p.parse_args()
    commands,page=build();page=bytearray(page)
    words=list(struct.unpack('<126I',commands))
    replaces={0x804:0x1000,0x808:256,0x80c:64,0xff4:64<<16,0xff8:64<<16,
              0xa00:0x42000000,0xa04:0x42000000,0xa0c:0x42000000,0xa10:0x42000000,
              0xc00:64<<16,0xc04:64<<16,0xe04:64<<16,0xe08:64<<16,0x1f04:0xdbf}
    for i in range(0,len(words),2):
        off=(words[i]&0xfff)*4
        if off in replaces:words[i+1]=replaces[off]
    # The second rectangle has its own input stream, same passthrough shader.
    i=next(i for i in range(0,len(words),2) if (words[i]&0xfff)*4==0x2044 and words[i+1]==0xe00)
    words[i]=0x20010000|0x1c08//4;words[i+1]=0xd60
    page[:504]=struct.pack('<126I',*words)
    outputs={'examples/gpu/demo-page.bin':bytes(page),
             'examples/gpu/demo-coordinates.bin':struct.pack('<65f',*(i/32-1 for i in range(65)))}
    for name,data in outputs.items():
        path=ROOT/name
        if a.check:assert path.read_bytes()==data,path
        else:path.write_bytes(data)
    print('64x64 demo:126 command words,4096 byte page,65 exact coordinate values')
if __name__=='__main__':main()
