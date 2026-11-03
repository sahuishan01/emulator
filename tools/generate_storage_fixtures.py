#!/usr/bin/env python3
"""Owned pitch/GOB RGBA, BC1 and BC3 pages with independent literal RGBA oracles."""
import argparse,struct
from pathlib import Path
from generate_texture_fixtures import build as texture_build
ROOT=Path(__file__).resolve().parent.parent
NAMES=['tiled-rgba','multigob-rgba','pitch-bc1','tiled-bc1-alpha','pitch-bc3','tiled-bc3-alpha']
PROFILES=[(4,4,8,0),(17,33,8,1),(7,5,0x24,None),(7,9,0x24,0),(5,7,0x26,None),(13,9,0x26,2)]
def pixel(i,x,y):
 if i<2:return [(255,0,0,255),(255,255,0,255),(0,255,0,255),(0,255,255,255)][(x+(3 if i==1 else 1)*y)%4]
 if i==2:return [(255,0,0,255),(0,255,0,255),(170,85,0,255),(85,170,0,255)][(x+y)%4]
 if i==3:return [(0,255,0,255),(255,0,0,255),(128,128,0,255),(0,0,0,0)][(x+2*y)%4]
 if i==4:
  c=[(0,255,0),(255,0,0),(85,170,0),(170,85,0)][(x+y)%4];return (*c,[255,0,219,182,146,109,73,36][(3*x+y)%8])
 c=[(0,0,255),(255,255,255),(85,85,255),(170,170,255)][(2*x+y)%4];return (*c,[20,200,56,92,128,164,0,255][(x+5*y)%8])
def source(i):
 w,h,fmt,log=PROFILES[i]
 if fmt==8:return bytes(c for y in range(h) for x in range(w) for c in pixel(i,x,y)),w*4,h
 out=bytearray()
 for by in range((h+3)//4):
  for bx in range((w+3)//4):
   xy=[(bx*4+x,by*4+y) for y in range(4) for x in range(4)]
   if i>=4:
    a,b=(255,0) if i==4 else (20,200)
    ai=sum((((3*x+y) if i==4 else (x+5*y))%8)<<(3*k) for k,(x,y) in enumerate(xy));out+=bytes([a,b])+ai.to_bytes(6,'little')
   endpoints=[(0xf800,0x07e0),(0x07e0,0xf800),(0x07e0,0xf800),(0x001f,0xffff)][i-2]
   ci=sum((((x+2*y) if i==3 else (2*x+y) if i==5 else x+y)%4)<<(2*k) for k,(x,y) in enumerate(xy))
   out+=struct.pack('<HHI',*endpoints,ci)
 return bytes(out),((w+3)//4)*(8 if fmt==0x24 else 16),(h+3)//4
def stored(data,row,rows,log):
 if log is None:
  pitch=(row+31)//32*32;out=bytearray([0xcd]*(pitch*rows))
  for y in range(rows):out[y*pitch:y*pitch+row]=data[y*row:(y+1)*row]
  return bytes(out),pitch//32
 bh=1<<log;nx=(row+63)//64;ny=(rows+8*bh-1)//(8*bh);out=bytearray([0xcd]*(nx*ny*bh*512))
 # Inverse byte-bit permutation, independently of the Rust forward address helper.
 for by in range(ny):
  for bx in range(nx):
   for gy in range(bh):
    base=((by*nx+bx)*bh+gy)*512
    for address in range(512):
     x=bx*64+(address&15)+((address>>1)&16)+((address>>3)&32)
     y=(by*bh+gy)*8+((address>>4)&1)+((address>>5)&6)
     if x<row and y<rows:out[base+address]=data[y*row+x]
 return bytes(out),log<<3
def build(i):
 commands,original=texture_build(0);page=bytearray(original);w,h,fmt,log=PROFILES[i]
 raw,row,rows=source(i);payload,config=stored(raw,row,rows,log)
 at=0x400 if len(payload)<=512 else 0x1000
 size=(max(4096,at+len(payload))+4095)//4096*4096;page+=bytes(size-len(page))
 page[0x400:0x600]=bytes(512);page[0x300:0x308]=struct.pack('<II',0x20010000|0x1214//4,0x40060000)
 page[at:at+len(payload)]=payload
 component=fmt|(2<<7)|(2<<10)|(2<<13)|(2<<16)|(2<<19)|(3<<22)|(4<<25)|(5<<28)
 struct.pack_into('<8I',page,0x600,component,at,((3 if log is not None else 2)<<21)|1,config,(1<<23)|(w-1),(1<<31)|(h-1),0,0)
 expected=bytes(c for y in range(h) for x in range(w) for c in pixel(i,x,y))
 return commands,bytes(page),expected,payload
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--check',action='store_true');a=p.parse_args()
 def save(p,b):
  if a.check:assert p.read_bytes()==b,p
  else:p.write_bytes(b)
 for i,n in enumerate(NAMES):
  commands,page,rgba,payload=build(i)
  for suffix,data in [('page.bin',page),('rgba',rgba),('source.bin',payload)]:save(ROOT/'examples/gpu'/f'storage-{n}-{suffix}',data)
  print(n,len(page),'page bytes',len(payload),'stored',len(rgba),'RGBA')
 save(ROOT/'examples/gpu/storage-profile.inc',f'.equ TEXTURED_COMMAND_WORDS, {len(build(0)[0])//4}\n'.encode())
if __name__=='__main__':main()
