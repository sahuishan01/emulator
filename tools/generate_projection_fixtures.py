#!/usr/bin/env python3
"""Author positive-W B197 command pages using only our original fixture assets."""
import argparse, struct
from pathlib import Path
from generate_texture_fixtures import build as texture
from generate_indexed_fixtures import build as indexed
from generate_depth_fixtures import depth_page
ROOT=Path(__file__).resolve().parent.parent
# Projected first-quad rectangle; transform.vert halves its NDC XY for draw2.
RECTS=[(4,2,12,6),(0,0,16,8),(0,0,16,8),(2,1,14,7),
       (0,0,16,8),(0,0,16,8),(2,0,14,8),(.125,.125,15.875,7.875),
       (0,0,16,8),(2,0,14,8),(0,0,16,8),(4,0,12,8)]
SCALES=[(2,)*4,(.5,)*4,(.5,1,2,4),(.5,1,2,4),
        (4,2,1,.5),(.5,1,2,4),(1.5,3,6,12),(.5,1,2,4),
        (.5,1,2,4),(1.5,3,6,12),(.5,1,2,4),(4,2,1,.5)]
def build(i):
 if i<8:commands,raw=texture(1 if i==4 else 0)
 elif i<12:commands,raw=indexed([0,1,2,0][i-8])
 else:commands,raw=depth_page([9,10,8,1][i-12])
 page=bytearray(raw)
 if i<12:
  x0,y0,x1,y1=RECTS[i];corners=[(x0,y0),(x1,y0),(x0,y1),(x1,y1)]
  indices=[0,1,2,2,1,3] if i<8 else list(range(4))
  start=0xd20 if i==10 else 0xd00
  for n,c in enumerate(indices):
   x,y=corners[c];w=SCALES[i][c]
   struct.pack_into('<4f',page,start+n*32,(x/8-1)*w,(y/4-1)*w,0.,w)
  if i==10:struct.pack_into('<4I',page,0xd00,0x7fc00000,0x7fc00000,0x7fc00000,0x7fc00000) # unused positive-base prefix
  if i==4:
   for a,b in [(1,2),(4,5)]:
    left=bytes(page[0xd00+a*32:0xd00+(a+1)*32]);page[0xd00+a*32:0xd00+(a+1)*32]=page[0xd00+b*32:0xd00+(b+1)*32];page[0xd00+b*32:0xd00+(b+1)*32]=left
  if i==9:
   # first-index2/u16/-1 base remain; reverse each triangle's winding.
   struct.pack_into('<6H',page,0xf04,1,3,2,3,4,2)
 else:
  for start in (0xd00,0x1300):
   for n,c in enumerate([0,1,2,2,1,3]):
    x,y,z,_=struct.unpack_from('<4f',page,start+n*16)
    w=(.5,1,2,4)[c] if i!=15 else 2.
    if i==15:struct.pack_into('<4f',page,start+n*16,x,y,z,w)
    else:struct.pack_into('<4f',page,start+n*16,x*w,y*w,z*w,w)
 # Right-only draw scissor, independent of full background clear.
 if i in (5,11):
  for at in range(0,len(commands),8):
   if (struct.unpack_from('<I',page,at)[0]&8191)*4==0xe04:struct.pack_into('<I',page,at+4,(16<<16)|8)
 commands=bytes(page[:len(commands)])
 return commands,bytes(page)
def main():
 p=argparse.ArgumentParser();p.add_argument('--check',action='store_true');a=p.parse_args()
 for i in range(16):
  commands,page=build(i)
  for name,data in [('commands',commands),('page',page)]:
   path=ROOT/'examples/gpu'/f'projection-{i}-{name}.bin'
   if a.check:assert path.read_bytes()==data,path
   else:path.write_bytes(data)
  print(f'projection-{i}: {len(commands)//4} words')
if __name__=='__main__':main()
