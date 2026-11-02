#!/usr/bin/env python3
"""Generate our own SM53 varying/texture shaders and B197/TIC/TSC command pages."""
import argparse,re,struct,subprocess,tempfile
from pathlib import Path
from generate_shader_fixtures import CONTROL,NOP,EXIT,imm,math
from generate_draw_fixtures import build as old_build,sph
ROOT=Path(__file__).resolve().parent.parent

def ipa(d,attr):return 0xe003ff800ff7ff00|(attr<<28)|d,f'IPA.PASS R{d}, a[0x{attr:x}], RZ'
def shaders():
 load=(0xefd9ff800807ff00,'ALD.128 R0, a[0x80]');store=(0xeff1ff800707ff00,'AST.128 a[0x70], R0')
 extra=(0xefd9ff800907ff04,'ALD.128 R4, a[0x90]');out=(0xeff1ff800807ff04,'AST.128 a[0x80], R4');end=(EXIT,'EXIT')
 return {'texture.vert':[load,store,extra,out,end],
 'texture-transform.vert':[load,imm(8,.5),math('FMUL',0,0,8),math('FMUL',1,1,8),store,extra,out,end],
 'texture.frag':[ipa(4,0x80),ipa(5,0x84),imm(8,0),(0xdeb80027a0870400,'TEX.B.LZ R0, R4, R8, 0x0, 2D, 0xf'),end],
 'varying.frag':[ipa(i,0x80+i*4) for i in range(4)]+[end]}
def shader_data(ops):
 ops=ops[:]
 while len(ops)%3:ops.append((NOP,'NOP'))
 words=[]
 for i in range(0,len(ops),3):words += [CONTROL]+[w for w,_ in ops[i:i+3]]
 return struct.pack('<'+'Q'*len(words),*words),ops

def header(vertex):
 w=list(struct.unpack('<20I',sph(vertex)))
 if vertex:w[6]=0xff;w[14]=15
 else:w[6]=15
 return struct.pack('<20I',*w)
def pixels(updated=False):
 palette=[(255,0,0,255),(255,255,0,255),(0,255,0,255),(0,255,255,255)]
 return bytes(c for y in range(4) for x in range(4) for c in palette[(x+y+int(updated))%4])
def build(index=0):
 old,_=old_build(1 if index==1 else 0)
 pairs=[((struct.unpack_from('<I',old,i)[0]&0x1fff)*4,struct.unpack_from('<I',old,i+4)[0]) for i in range(0,len(old),8)]
 pairs=[(o,0x1020 if o==0x1c00 else 0xdc0-1 if o==0x1f04 else v) for o,v in pairs]
 at=next(i for i,(o,_) in enumerate(pairs) if o==0x2040)
 pairs[at:at]=[(0x1164,0x38200800),(0xf10,1),(0x1574,1),(0x1578,0x600),(0x157c,0),(0x155c,1),(0x1560,0x700),(0x1564,0)]
 commands=b''.join(struct.pack('<II',0x20010000|o//4,v) for o,v in pairs)
 page=bytearray(4096);page[:len(commands)]=commands
 page[len(commands):len(commands)+8]=struct.pack('<II',0x2001048d,0)
 page[0x400:0x408]=struct.pack('<II',0x20010000|0x1214//4,0x40060000)
 for offset,name,vertex in [(0xa00,'texture.vert',True),(0xb00,'texture.frag',False),(0xc00,'texture.frag',False),(0xe00,'texture-transform.vert',True)]:
  raw,_=shader_data(shaders()[name]);data=header(vertex)+raw;assert len(data)<=256;page[offset:offset+len(data)]=data
 verts=[(-1.,-1.),(1.,-1.),(-1.,1.),(-1.,1.),(1.,-1.),(1.,1.)]
 for i,(x,y) in enumerate(verts):
  uv=(1.5*(x+1)/2-.25,1.5*(y+1)/2-.25)
  struct.pack_into('<8f',page,0xd00+i*32,x,y,0.,1.,*uv,0.,1.)
 rgba=pixels(index==2)
 for y in range(4):page[0x500+y*32:0x500+y*32+16]=rgba[y*16:y*16+16]
 component=8|(2<<7)|(2<<10)|(2<<13)|(2<<16)|(2<<19)|(3<<22)|(4<<25)|(5<<28)
 struct.pack_into('<8I',page,0x600,component,0x500,(2<<21)|1,1,(1<<23)|3,(1<<31)|3,0,0)
 struct.pack_into('<8I',page,0x700,0x92,0x51,0,0,0,0,0,0)
 assert len(commands)+8<=0x400
 return commands,bytes(page)

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--check',action='store_true');p.add_argument('--nvdisasm',type=Path);p.add_argument('--validate-spirv',action='store_true');a=p.parse_args()
 def save(path,data):
  if a.check:assert path.read_bytes()==data,path
  else:path.write_bytes(data)
 for name,ops in shaders().items():
  raw,listing=shader_data(ops);path=ROOT/'examples/shaders'/f'{name}.bin';save(path,raw);save(path.with_suffix('.sass'),('\n'.join(t for _,t in listing)+'\n').encode())
  if a.nvdisasm:
   r=subprocess.run([str(a.nvdisasm.resolve()),'-b','SM53','-hex',str(path)],capture_output=True,text=True,check=True)
   actual=re.findall(r'/\*[0-9a-f]+\*/\s+(.+?);',r.stdout);assert actual==[t for _,t in listing],(name,actual,listing)
  print(name,len(raw),'bytes'+(' / NVIDIA exact disassembly' if a.nvdisasm else ''))
 for index,name in enumerate(['rgba','bgra','updated']):
  commands,page=build(index)
  for suffix,data in [('commands',commands),('page',page)]:save(ROOT/'examples/gpu'/f'texture-{name}-{suffix}.bin',data)
  print('texture-'+name,len(commands)//4,'words')
 count=len(build()[0])//4
 save(ROOT/'examples/gpu/texture-profile.inc',f'.equ TEXTURED_COMMAND_WORDS, {count}\n'.encode())
 if a.validate_spirv:
  with tempfile.TemporaryDirectory(prefix='emulator-texture-spirv-') as temporary:
   subprocess.run(['cargo','run','--offline','-q','-p','emulator-shader','--example','translate_textures','--',temporary],check=True,cwd=ROOT)
   modules=sorted(Path(temporary).glob('*.spv'));assert len(modules)==4
   for module in modules:subprocess.run(['spirv-val','--target-env','vulkan1.1',str(module)],check=True)
  print('Four original texture/varying modules passed Vulkan 1.1 SPIR-V validation')
if __name__=='__main__':main()
