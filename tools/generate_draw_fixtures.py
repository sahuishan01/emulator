#!/usr/bin/env python3
"""Generate owned B197 draw pages with original SPH/SASS/vertex payloads."""
import argparse
from pathlib import Path
import struct
ROOT=Path(__file__).resolve().parent.parent

def sph(vertex):
    w=[0]*20
    w[0]=(1|(3<<5)|(1<<10)|(1<<17)|(1<<26)) if vertex else (2|(3<<5)|(5<<10)|(1<<17))
    w[4]=1<<12
    if vertex:
        w[6]=15
        w[13]=0xf000
    else:
        w[18]=15
    return struct.pack('<20I',*w)

def build(index=0):
    one=0x3f800000
    m=[(0,0xb197),(0x800,1),(0x804,0x800),(0x808,64),(0x80c,8),(0x810,0xcf if index==1 else 0xd5),
       (0x814,0x1000),(0x818,1),(0x81c,0),(0x820,0),(0xff4,16<<16),(0xff8,8<<16),
       (0x10f8,0),(0xd80,0),(0xd84,0),(0xd88,0),(0xd8c,one),(0x19d0,0x3c)]
    m += [(0xa00,0x41000000),(0xa04,0x40800000),(0xa08,one),(0xa0c,0x41000000),(0xa10,0x40800000),(0xa14,0),
          (0xc00,16<<16),(0xc04,8<<16),(0xc08,0),(0xc0c,one),(0xe00,1),(0xe04,16<<16),(0xe08,8<<16),
          (0x12cc,0),(0x1360,0),(0x1380,0),(0x1918,0),(0x13ac,0),(0x192c,1),(0x1a00,0x1111),(0x121c,1),
          (0x1c00,0x1010),(0x1c04,1),(0x1c08,0xd00),(0x1c0c,0),(0x1f00,1),(0x1f04,0xd5f),(0x1160,0x38200000),
          (0x1608,1),(0x160c,0)]
    m += [(0x2000+i*64,0) for i in [0,2,3,4]]
    m += [(0x2040,0x11),(0x2044,0xa00),(0x204c,16),(0x2140,0x51),(0x2144,0xb00),(0x214c,16),
          (0x1214,0x40060000),(0x2044,0xe00),(0x2144,0xc00),(0xe04,(8 if index==2 else 16)<<16),(0x1214,0x40060000)]
    commands=b''.join(struct.pack('<II',0x20010000|(off//4),value) for off,value in m)
    assert len(commands)+8<=0x400
    page=bytearray(4096);page[:len(commands)]=commands
    page[len(commands):len(commands)+8]=struct.pack('<II',0x2001048d,0)
    page[0x400:0x408]=struct.pack('<II',0x20010000|0x1214//4,0x40060000)
    for offset,name,vertex in [(0xa00,'passthrough.vert',True),(0xb00,'green.frag',False),(0xc00,'red.frag',False),(0xe00,'transform.vert',True)]:
        shader=sph(vertex)+(ROOT/'examples/shaders'/f'{name}.bin').read_bytes()
        assert len(shader)<=256
        page[offset:offset+len(shader)]=shader
    verts=[-1.,-1.,0.,1.,1.,-1.,0.,1.,-1.,1.,0.,1.,-1.,1.,0.,1.,1.,-1.,0.,1.,1.,1.,0.,1.]
    page[0xd00:0xd60]=struct.pack('<24f',*verts)
    return commands,bytes(page)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--check',action='store_true');a=p.parse_args()
    for index,name in enumerate(['rgba','bgra','scissor']):
        commands,page=build(index)
        for suffix,data in [('commands',commands),('page',page)]:
            path=ROOT/'examples/gpu'/f'draw-{name}-{suffix}.bin'
            if a.check: assert path.read_bytes()==data,path
            else:path.write_bytes(data)
        print(f'draw-{name}: {len(commands)//4} words / {len(page)} byte page')
if __name__=='__main__':main()
