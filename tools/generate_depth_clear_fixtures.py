#!/usr/bin/env python3
"""Original B197 depth-clear corpus; all existing draw/shader assets are ours."""
import argparse,struct
from pathlib import Path
from generate_depth_fixtures import depth_page
ROOT=Path(__file__).resolve().parent.parent
# base depth fixture, initial raw depth, (before draw, rectangle, value, scissor).
FULL=(0,0,16,8)
PROFILES=[
 (1,0.,[(0,FULL,1.,None)]),(1,1.,[(0,FULL,0.,None)]),
 (1,0.,[(0,(8,0,16,8),1.,None)]),(1,0.,[(0,(2,1,14,7),1.,(8,0,16,8))]),
 (1,1.,[(0,(16,0,16,8),0.,None)]),
 (1,0.,[(0,FULL,1.,None),(0,(0,0,8,8),0.,None)]),
 (1,1.,[(1,FULL,1.,None)]),(1,1.,[(1,FULL,0.,None)]),
 (1,1.,[(2,FULL,.5,None)]),(1,1.,[(1,(8,0,16,8),1.,None)]),
 (8,0.,[(0,FULL,1.,None)]),(0,1.,[(0,FULL,.5,None)]),
 (4,1.,[(0,FULL,0.,None)]),(3,0.,[(0,FULL,.5,None)]),
 (1,0.,[(0,(15,7,16,8),1.,None)]),
 (1,0.,[(0,FULL,1.,None),(2,(4,2,12,6),.875,None)])]
def clear_page(index):
 base,initial,clears=PROFILES[index];commands,old=depth_page(base);page=bytearray(old)
 pairs=[(struct.unpack_from('<I',commands,i)[0]&8191,struct.unpack_from('<I',commands,i+4)[0]) for i in range(0,len(commands),8)]
 out=[];draw=0;write=0 if base==8 else 1
 def inject(when):
  for before,rect,value,scissor in clears:
   if before!=when:continue
   x,y,x2,y2=rect
   out.extend([(0x7f8,0),(0x10f8,0x110 if scissor else 0x10),(0xd6c,x|(x2<<16)),(0xd70,y|(y2<<16)),(0xd90,struct.unpack('<I',struct.pack('<f',value))[0])])
   if scissor:
    a,b,c,d=scissor;out.extend([(0xe00,1),(0xe04,a|(c<<16)),(0xe08,b|(d<<16))])
   out.extend([(0x19d0,1),(0x10f8,0),(0xe00,1),(0xe04,16<<16),(0xe08,8<<16),(0x7f8,1-write)])
 for word,value in pairs:
  offset=word*4
  if offset==0x1214:inject(draw);draw+=1
  out.append((offset,value))
 inject(2)
 commands=b''.join(struct.pack('<II',0x20010000|offset//4,value) for offset,value in out)
 assert len(commands)<0x800
 page[:0x800]=bytes(0x800);page[:len(commands)]=commands
 page[0x1000:0x1200]=struct.pack('<128f',*[initial]*128)
 return commands,bytes(page)
def main():
 p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
 for i in range(len(PROFILES)):
  commands,page=clear_page(i)
  for suffix,data in [('commands',commands),('page',page)]:
   path=ROOT/'examples/gpu'/f'depth-clear-{i}-{suffix}.bin'
   if a.check:assert path.read_bytes()==data,path
   else:path.write_bytes(data)
  print(f'depth-clear-{i}: {len(commands)//4} words')
if __name__=='__main__':main()
