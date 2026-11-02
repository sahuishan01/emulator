#!/usr/bin/env python3
"""Exercise the visible homebrew via fresh UI bounds and retain screen/report evidence."""
import argparse,json,re,struct,subprocess,time,xml.etree.ElementTree as ET
from pathlib import Path

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apk',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--serial',default='emulator-5580');p.add_argument('--hold-controls',action='store_true');a=p.parse_args()
 assert a.serial=='emulator-5580','Only the owned AVD is authorized by this runner'
 out=a.output;out.mkdir(parents=True,exist_ok=True)
 adb=[str(Path.home()/'Android/Sdk/platform-tools/adb'),'-s',a.serial];pkg='com.algosculptor.emulator.gpulab';notes=[]
 def call(*args,timeout=30):return subprocess.run(adb+list(args),check=True,capture_output=True,timeout=timeout).stdout
 def ui(phase):
  for retry in range(3):
   try:
    remote=f'/sdcard/homebrew-{phase}-{retry}.xml'
    result=call('shell','uiautomator','dump',remote)
    if b'dumped to:' not in result:raise ET.ParseError('UI dump did not complete; refusing a stale tree')
    raw=call('exec-out','cat',remote);root=ET.fromstring(raw);(out/f'{phase}-{retry}.xml').write_bytes(raw)
    if any(n.get('text','')=="System UI isn't responding" for n in root.iter('node')):
     n=next(n for n in root.iter('node') if n.get('text','').lower()=='wait');tap_node(n);notes.append('System UI startup ANR Wait');continue
    assert not any("isn't responding" in n.get('text','') for n in root.iter('node')),'App/system ANR'
    return root
   except (subprocess.SubprocessError,ET.ParseError) as e:notes.append(f'{phase} XML retry:{e}')
  raise RuntimeError('No fresh UI:'+phase)
 def bounds(n):return list(map(int,re.findall(r'\d+',n.get('bounds'))))
 def tap_node(n):
  x,y,x2,y2=bounds(n);call('shell','input','tap',str((x+x2)//2),str((y+y2)//2))
 def tap(label,phase):
  r=ui(phase);n=next(n for n in r.iter('node') if n.get('text','').lower()==label.lower());tap_node(n);return r
 def report():return json.loads(call('exec-out','run-as',pkg,'cat','files/last-demo-report.json'))
 def wait(predicate,phase,seconds=35):
  end=time.monotonic()+seconds
  while time.monotonic()<end:
   try:
    x=report()
    if x.get('error'):raise RuntimeError(x['error'])
    if predicate(x):
     (out/f'{phase}.json').write_text(json.dumps(x,indent=2));print('Verified',phase,{k:x.get(k) for k in ['frame','paddleX','score','presentedFrames']},flush=True);return x
   except (subprocess.SubprocessError,json.JSONDecodeError):pass
   time.sleep(.3)
  raise RuntimeError('Timed out:'+phase)
 def screen(name): (out/f'{name}.png').write_bytes(call('exec-out','screencap','-p'))
 try:
  call('install','-r',str(a.apk),timeout=60);call('shell','am','force-stop',pkg);call('shell','run-as',pkg,'rm','-f','files/last-demo-report.json');call('logcat','-c');call('shell','am','start','-W','-n',pkg+'/.HomeActivity')
  tap('Play Catch','open-demo')
  first=wait(lambda x:x.get('running') and x.get('presentedFrames',0)>10,'first-frames')
  assert first['renderWidth']==64 and first['renderHeight']==64 and first['frameAllocationCount']==4
  if a.hold_controls:
   def control_center(label,phase):
    r=ui(phase);n=next(n for n in r.iter('node') if n.get('text','').lower()==label.lower());x,y,x2,y2=bounds(n);return (x+x2)//2,(y+y2)//2
   def center_paddle(phase,value=24):
    r=ui(phase);n=next(n for n in r.iter('node') if n.get('content-desc')=='Homebrew game screen');x,y,x2,y2=bounds(n)
    call('shell','input','tap',str(x+round((x2-x)*(value+8)/64)),str((y+y2)//2));return wait(lambda q:q.get('paddleX')==value,phase+'-centered')
   def hold(label,duration,phase):
    x,y=control_center(label,phase);call('shell','input','touchscreen','motionevent','DOWN',str(x),str(y))
    try:time.sleep(duration/1000)
    finally:call('shell','input','touchscreen','motionevent','UP',str(x),str(y))
   def released(value,phase):
    # The capture file can still describe a frame during the hold. Allow its
    # 500ms cadence to settle after UP, then compare two later guest frames.
    time.sleep(.9);baseline=report();time.sleep(1.2);q=report()
    assert q['paddleX']==baseline['paddleX'],(phase,baseline,q)
    assert q['frame']>baseline['frame'],(phase,'Guest stopped')
    (out/(phase+'.json')).write_text(json.dumps(q,indent=2))
   center_paddle('hold-start',0)
   hold('Right',400,'hold-right')
   right=wait(lambda q:8<=q.get('paddleX',0)<48,'right-repeats');released(right['paddleX'],'right-release-stops')
   center_paddle('left-hold-start',48)
   hold('Left',400,'hold-left')
   left=wait(lambda q:0<q.get('paddleX',0)<=40,'left-repeats');released(left['paddleX'],'left-release-stops')
   hold('Right',1300,'right-edge');wait(lambda q:q.get('paddleX')==48,'right-clamped')
   hold('Left',1300,'left-edge');wait(lambda q:q.get('paddleX')==0,'left-clamped')
   center_paddle('cancel-start')
   x,y=control_center('Left','cancel-gesture');r=ui('cancel-destination');screen_node=next(n for n in r.iter('node') if n.get('content-desc')=='Homebrew game screen');end=bounds(screen_node)[2]-4
   call('shell','input','touchscreen','motionevent','DOWN',str(x),str(y))
   try:
    call('shell','input','touchscreen','motionevent','MOVE',str(end),str(y));time.sleep(.7)
   finally:call('shell','input','touchscreen','motionevent','UP',str(end),str(y))
   cancelled=wait(lambda q:0<q.get('paddleX',24)<24,'leaving-button-cancels');released(cancelled['paddleX'],'cancel-remains-stopped')
   center_paddle('focus-start');x,y=control_center('Right','held-background')
   call('shell','input','touchscreen','motionevent','DOWN',str(x),str(y));call('shell','input','keyevent','3');call('shell','input','touchscreen','motionevent','UP',str(x),str(y))
   stopped=wait(lambda q:q.get('running') is False,'held-background-stopped')
   call('shell','am','start','-W','-n',pkg+'/.HomeActivity');r=ui('held-foreground')
   if any(n.get('text','').lower()=='play catch' for n in r.iter('node')):tap('Play Catch','held-reopen')
   q=wait(lambda q:q.get('running') and q.get('capturedAtEpochMs',0)>stopped['capturedAtEpochMs'],'held-foreground-ready');released(q['paddleX'],'held-input-cleared-on-background')
   center_paddle('after-holds')
  tap('Pause','pause')
  paused=wait(lambda x:x.get('paused') is True,'paused')
  time.sleep(1);assert report()['frame']==paused['frame'],'Guest advanced while paused'
  screen('visible-paused')
  # Verify actual displayed foreground colors, in addition to internal Vulkan readback.
  r=ui('surface-paused')
  n=next(n for n in r.iter('node') if n.get('content-desc')=='Homebrew game screen');x,y,x2,y2=bounds(n)
  raw=call('exec-out','screencap');width,height,format=struct.unpack_from('<3I',raw);header=len(raw)-width*height*4
  assert format in (1,2) and header in (12,16),'Unexpected Android raw screenshot format'
  colors=set()
  for row in range(y,y2):
   data=raw[header+(row*width+x)*4:header+(row*width+x2)*4]
   colors.update(tuple(data[i:i+3]) for i in range(0,len(data),4))
  assert (0,255,0) in colors and (255,0,0) in colors and (0,0,0) in colors,colors
  (out/'visible-color-evidence.json').write_text(json.dumps({'bounds':[x,y,x2,y2],'rawFormat':format,'headerBytes':header,'green':(0,255,0) in colors,'red':(255,0,0) in colors,'black':(0,0,0) in colors},indent=2))
  tap('Right','move-right-paused');tap('Resume','resume-after-right')
  moved=wait(lambda x:x.get('paddleX')==28 and x.get('frame',0)>paused['frame'],'right-input')
  # Drag from fresh surface bounds to the far left; left edge means paddle x0.
  r=ui('drag-input');n=next(n for n in r.iter('node') if n.get('content-desc')=='Homebrew game screen');x,y,x2,y2=bounds(n)
  call('shell','input','swipe',str((x+x2)//2),str((y+y2)//2),str(x+max(2,(x2-x)//64)),str((y+y2)//2),'400')
  drag=wait(lambda q:q.get('paddleX')==0 and q.get('frame',0)>moved['frame'],'drag-left')
  tap('Reset','reset');reset=wait(lambda q:q.get('frame',99999)<drag['frame'] and q.get('score')==0 and q.get('misses')==0,'reset-state')
  # Reset starts ball x20; choose paddle20 deterministically using key events.
  call('shell','input','keyevent','22')
  wait(lambda q:q.get('paddleX')==4,'first-dpad-press')
  for _ in range(4):call('shell','input','keyevent','22')
  score=wait(lambda q:q.get('paddleX')==20 and q.get('score',0)>=1,'guest-catch',seconds=50)
  screen('visible-catch');assert score['completedFrames']==score['presentedFrames'] and score['validationErrors']==0
  # HOME must stop the worker/session; resume starts a fresh guest and swapchain.
  call('shell','input','keyevent','3');stopped=wait(lambda q:q.get('running') is False,'background-stopped')
  time.sleep(1);assert report()['completedFrames']==stopped['completedFrames'],'Background rendering continued'
  call('shell','am','start','-W','-n',pkg+'/.HomeActivity')
  # Existing task may resume DemoActivity, or MainActivity may come to front.
  r=ui('return-to-app')
  if any(n.get('text','').lower()=='play catch' for n in r.iter('node')):tap('Play Catch','reopen-demo')
  resumed=wait(lambda q:q.get('running') and q.get('frame',0)>5 and q.get('capturedAtEpochMs',0)>stopped['capturedAtEpochMs'],'foreground-recreated')
  for i in range(2):
   tap('Back to library',f'back-{i}');wait(lambda q:q.get('running') is False,f'closed-{i}')
   tap('Play Catch',f'reopen-{i}');wait(lambda q:q.get('running') and q.get('frame',0)>5,f'reopened-{i}')
  tap('Pause','final-pause');final=wait(lambda q:q.get('paused') is True,'final-demo');screen('visible-final')
  tap('Back to library','finish-demo');closed=wait(lambda q:q.get('running') is False,'final-closed')
  assert closed['error']=='' and closed['frameAllocationCount']==4
  evidence={'passed':True,'checks':['visible Vulkan ball/paddle pixels','guest animation','pause','button input','drag input','reset','guest collision/score','GPU completion equals presentation','background stop','foreground recreation','repeated surface/session teardown','bounded per-frame allocations'],'notes':notes,'first':first,'last':closed}
  if a.hold_controls:evidence['checks']+=['held left/right repeat','release stops before edge','both edge clamps','moving outside cancels','held input cleared on background']
  (out/'verification.json').write_text(json.dumps(evidence,indent=2));print(json.dumps({'passed':True,'checks':evidence['checks'],'notes':notes}),flush=True)
 finally:
  for name,args in [('logcat.txt',['logcat','-d']),('crash-buffer.txt',['logcat','-b','crash','-d']),('exit-info.txt',['shell','dumpsys','activity','exit-info',pkg])]:
   try:(out/name).write_bytes(call(*args))
   except subprocess.SubprocessError:pass
if __name__=='__main__':main()
