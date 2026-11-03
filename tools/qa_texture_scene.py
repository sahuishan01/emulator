#!/usr/bin/env python3
"""Exercise the real guest texture scene, displayed pixels and surface lifetime."""
import argparse,json,re,struct,subprocess,time,xml.etree.ElementTree as ET
from pathlib import Path
from qa_pattern_pixels import expected_pixel

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apk',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 out=a.output;out.mkdir(parents=True,exist_ok=True);adb=[str(Path.home()/'Android/Sdk/platform-tools/adb'),'-s','emulator-5580'];pkg='com.algosculptor.emulator.gpulab';notes=[]
 def call(*args,timeout=30):return subprocess.run(adb+list(args),check=True,capture_output=True,timeout=timeout).stdout
 def bounds(n):return tuple(map(int,re.findall(r'\d+',n.get('bounds'))))
 def tapnode(n):
  x,y,x2,y2=bounds(n);call('shell','input','tap',str((x+x2)//2),str((y+y2)//2))
 def ui(phase):
  for retry in range(3):
   remote=f'/sdcard/texture-{phase}-{retry}.xml';result=call('shell','uiautomator','dump',remote)
   if b'dumped to:' not in result:continue
   raw=call('exec-out','cat',remote);root=ET.fromstring(raw);(out/f'{phase}-{retry}.xml').write_bytes(raw)
   if any(n.get('text')=="System UI isn't responding" for n in root.iter('node')):
    tapnode(next(n for n in root.iter('node') if n.get('text','').lower()=='wait'));notes.append('System UI startup Wait');continue
   assert not any("isn't responding" in n.get('text','') for n in root.iter('node'))
   return root
  raise RuntimeError('Fresh XML unavailable:'+phase)
 def button(text,phase):
  root=ui(phase);return next(n for n in root.iter('node') if n.get('text')==text)
 def tap(text,phase):tapnode(button(text,phase))
 def report():return json.loads(call('exec-out','run-as',pkg,'cat','files/last-texture-report.json'))
 def wait(variant,running,after,phase):
  end=time.monotonic()+40
  while time.monotonic()<end:
   try:
    r=report()
    if r.get('error'):raise RuntimeError(r['error'])
    if r.get('variant')==variant and r.get('running')==running and r.get('capturedAtEpochMs',0)>after:
     assert r['pixelsVerified'] and r['completedFrames']==r['presentedFrames']==1 and r['validationErrors']==0
     assert r['renderWidth']==16 and r['renderHeight']==8
     (out/f'{phase}.json').write_text(json.dumps(r,indent=2));print('Verified',phase,flush=True);return r
   except (subprocess.SubprocessError,json.JSONDecodeError):pass
   time.sleep(.2)
  raise RuntimeError('Scene did not reach expected state:'+phase)
 def open_scene(phase):
  call('shell','am','start','-W','-f','0x10008000','-n',pkg+'/.HomeActivity');r=ui(phase+'-library')
  edit=next(n for n in r.iter('node') if n.get('class')=='android.widget.EditText');tapnode(edit);call('shell','input','text','texture');call('shell','input','keyevent','4');tap('Open scene',phase+'-open')
 def pixels(variant,phase):
  r=ui(phase);n=next(n for n in r.iter('node') if n.get('content-desc')=='Textured guest scene');x,y,x2,y2=bounds(n);w,h=x2-x,y2-y
  dw=w;dh=max(1,dw*8//16)
  if dh>h:dh=h;dw=max(1,dh*16//8)
  left=x+(w-dw)//2;top=y+(h-dh)//2
  raw=call('exec-out','screencap');width,height,fmt=struct.unpack_from('<3I',raw);header=len(raw)-width*height*4;assert fmt in(1,2) and header in(12,16)
  for py in range(8):
   for px in range(16):
    expected=expected_pixel(variant,px,py)
    xx=left+(2*px+1)*dw//32;yy=top+(2*py+1)*dh//16;at=header+(yy*width+xx)*4
    assert tuple(raw[at:at+4])==expected,(phase,px,py,tuple(raw[at:at+4]),expected)
  (out/f'{phase}.png').write_bytes(call('exec-out','screencap','-p'))
  print('Verified',phase,'128 displayed pixel centers',flush=True)
 try:
  call('install','-r',str(a.apk),timeout=60);call('shell','am','force-stop',pkg);call('logcat','-c')
  open_scene('initial');q=wait(0,True,0,'rgba-ready');pixels(0,'rgba-display')
  for variant in [*range(1,12),0]:
   previous=q['capturedAtEpochMs'];tap('Next pattern','next-'+str(variant));q=wait(variant,True,previous,'variant-'+str(variant));pixels(variant,'display-'+str(variant))
  n=button('Next pattern','rapid-patterns');baseline=q['capturedAtEpochMs']
  for _ in range(12):tapnode(n)
  q=wait(0,True,baseline,'rapid-pattern-final');pixels(0,'rapid-display')
  for variant in range(1,12):
   baseline=q['capturedAtEpochMs'];tap('Next pattern','indexed-lifecycle-'+str(variant));q=wait(variant,True,baseline,'indexed-lifecycle-ready-'+str(variant))
  baseline=q['capturedAtEpochMs'];call('shell','settings','put','system','accelerometer_rotation','0');call('shell','settings','put','system','user_rotation','1')
  q=wait(11,True,baseline,'landscape-recreated');pixels(11,'landscape-display')
  baseline=q['capturedAtEpochMs'];call('shell','settings','put','system','user_rotation','0');q=wait(11,True,baseline,'portrait-recreated')
  baseline=q['capturedAtEpochMs'];call('shell','input','keyevent','3');stopped=wait(11,False,baseline,'background-closed')
  time.sleep(.8);assert report()['completedFrames']==stopped['completedFrames']
  open_scene('foreground');q=wait(0,True,stopped['capturedAtEpochMs'],'foreground-reopened');pixels(0,'foreground-display')
  for i in range(2):
   baseline=q['capturedAtEpochMs'];tap('Back to library','close-'+str(i));stopped=wait(0,False,baseline,'closed-'+str(i));open_scene('reopen-'+str(i));q=wait(0,True,stopped['capturedAtEpochMs'],'reopened-'+str(i))
  tap('Back to library','final-back');wait(0,False,q['capturedAtEpochMs'],'final-closed')
  assert not call('logcat','-b','crash','-d').strip()
  assert not re.search(rb'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab',call('logcat','-d'))
  result={'passed':True,'checks':['library scene launch','real RGBA sampled pixels','BGRA rendering preserves displayed colors','changed source texture pixels','byte indexed quad pixels','short indexed negative-base pixels','u32 indexed triangle coverage','tiled RGBA pixels','multi-GOB odd-dimension pixels','pitched BC1 pixels','tiled BC1 alpha pixels','pitched BC3 alpha pixels','tiled BC3 alpha pixels','pattern wrap','rapid input coalescing','landscape surface recreation','portrait surface recreation','background teardown','foreground reopen','repeated native/surface teardown'],'displaySamplesPerCapture':128,'notes':notes}
  (out/'verification.json').write_text(json.dumps(result,indent=2));print(json.dumps(result),flush=True)
 finally:
  try:call('shell','settings','put','system','user_rotation','0');call('shell','settings','put','system','accelerometer_rotation','1')
  except subprocess.SubprocessError:pass
  for name,args in [('logcat.txt',['logcat','-d']),('crash-buffer.txt',['logcat','-b','crash','-d']),('exit-info.txt',['shell','dumpsys','activity','exit-info',pkg])]:
   try:(out/name).write_bytes(call(*args))
   except subprocess.SubprocessError:pass
if __name__=='__main__':main()
