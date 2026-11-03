#!/usr/bin/env python3
"""Generate original B197 depth command pages, reusing only our owned draw assets."""
import argparse
import struct
from pathlib import Path
from generate_draw_fixtures import build
ROOT=Path(__file__).resolve().parent.parent
# compare index, initial value, first Z, second Z, write enable
PROFILES=[(0,1.,.25,.75,1),(1,1.,.25,.75,1),(2,.5,.5,.25,1),
          (3,1.,.5,.5,1),(4,0.,.75,.25,1),(5,1.,.25,.25,1),
          (6,0.,.5,.5,1),(7,1.,.25,.75,1),(1,1.,.25,.75,0)]
def depth_page(index):
    commands,original=build(0)
    function,initial,z0,z1,write=PROFILES[index]
    page=bytearray(original)+bytearray(4096)
    pairs=[(struct.unpack_from('<I',commands,i)[0]&8191,struct.unpack_from('<I',commands,i+4)[0]) for i in range(0,len(commands),8)]
    methods=[];first=True
    for word,value in pairs:
        offset=word*4
        if offset==0x1214:
            if first:
                methods.extend([(0x12cc,1),(0x7f8,1-write),(0xfe0,1),(0xfe4,0x1000),(0xfe8,0xa),(0xfec,0),(0xff0,0),
                    (0x1228,16),(0x122c,8),(0x1230,1),(0x1208,0),(0x1538,1),(0x179c,0),(0x12e8,write),(0x130c,0x200+function),(0x19bc,0)])
                first=False
            else:methods.extend([(0x1c08,0x1300),(0x1f04,0x135f)])
        methods.append((offset,value))
    commands=b''.join(struct.pack('<II',0x20010000|offset//4,value) for offset,value in methods)
    assert len(commands)<0x400
    page[:0x400]=bytes(0x400);page[:len(commands)]=commands
    first_vertices=bytearray(original[0xd00:0xd60]);second_vertices=bytearray(first_vertices)
    for i in range(6):
        struct.pack_into('<f',first_vertices,i*16+8,z0)
        struct.pack_into('<f',second_vertices,i*16+8,z1)
    page[0xd00:0xd60]=first_vertices;page[0x1300:0x1360]=second_vertices
    page[0x1000:0x1200]=struct.pack('<128f',*[initial]*128)
    return commands,bytes(page)
def main():
    p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
    for i in range(len(PROFILES)):
        commands,page=depth_page(i)
        for name,data in [('commands',commands),('page',page)]:
            path=ROOT/'examples/gpu'/f'depth-{i}-{name}.bin'
            if a.check:assert path.read_bytes()==data,path
            else:path.write_bytes(data)
        print(f'depth-{i}: {len(commands)//4} words')
if __name__=='__main__':main()
