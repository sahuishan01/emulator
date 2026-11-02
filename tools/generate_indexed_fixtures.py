#!/usr/bin/env python3
"""Build owned B197 byte/u16/u32 indexed draws from our texture/shader fixture."""
import argparse, struct
from pathlib import Path
from generate_texture_fixtures import build as texture_build
ROOT = Path(__file__).resolve().parent.parent

def build(index=0):
    commands, original = texture_build(2 if index == 2 else 0)
    page = bytearray(original)
    width, method, first, base = [(1,0x17ec,0,0),(2,0x17e8,2,-1),(4,0x17e4,1,1)][index]
    pairs = [((struct.unpack_from('<I',commands,i)[0]&0x1fff)*4,struct.unpack_from('<I',commands,i+4)[0]) for i in range(0,len(commands),8)]
    values = [v-min(base,0) for v in [0,1,2,2,1,3]]
    at = next(i for i,(o,_) in enumerate(pairs) if o==0x2040)
    pairs[at:at] = [(0x17c8,1),(0x17cc,0xf00),(0x17d0,1),(0x17d4,0xf00+(first+6)*width-1),(0x1434,base&0xffffffff),(0xde8,0)]
    seen = 0
    for i,(o,v) in enumerate(pairs):
        if o == 0x1f04: pairs[i]=(o,0xd00+(4+int(base>0))*32-1)
        if o == 0x1214:
            count = 3 if index==2 and seen else 6
            pairs[i]=(method,0x40000000|(count<<16)|first); seen+=1
    commands = b''.join(struct.pack('<II',0x20010000|o//4,v) for o,v in pairs)
    page[:0x400]=bytes(0x400);page[:len(commands)]=commands
    page[len(commands):len(commands)+8]=struct.pack('<II',0x2001048d,0)
    count=3 if index==2 else 6
    page[0x400:0x408]=struct.pack('<II',0x20010000|method//4,0x40000000|(count<<16)|first)
    page[0xd00:0xe00]=bytes(0x100)
    if base>0:struct.pack_into('<8f',page,0xd00,0,0,0,1,0,0,0,1)
    for i,source in enumerate([0,1,2,5]):
        dest=0xd00+(i+int(base>0))*32
        page[dest:dest+32]=original[0xd00+source*32:0xd00+(source+1)*32]
    for i,v in enumerate([0xff]*first+values):
        page[0xf00+i*width:0xf00+(i+1)*width]=v.to_bytes(width,'little')
    assert len(commands)+8<=0x400
    return commands,bytes(page)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--check',action='store_true');a=p.parse_args()
    def save(path,data):
        if a.check:assert path.read_bytes()==data,path
        else:path.write_bytes(data)
    for i,name in enumerate(['u8','u16','u32']):
        commands,page=build(i)
        for suffix,data in [('commands',commands),('page',page)]:save(ROOT/'examples/gpu'/f'indexed-{name}-{suffix}.bin',data)
        print(f'indexed-{name}: {len(commands)//4} words, four shared corners'+(' plus base vertex padding' if i==2 else ''))
    save(ROOT/'examples/gpu/indexed-profile.inc',f'.equ TEXTURED_COMMAND_WORDS, {len(build()[0])//4}\n'.encode())
if __name__=='__main__':main()
