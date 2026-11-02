#!/usr/bin/env python3
"""Test the real library/SAF/import/runtime/settings flows with a separate QA provider."""
import argparse,json,re,sqlite3,subprocess,time,xml.etree.ElementTree as ET
from pathlib import Path

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apk',type=Path,required=True);p.add_argument('--provider-apk',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--serial',default='emulator-5580');a=p.parse_args()
 assert a.serial=='emulator-5580'
 out=a.output;out.mkdir(parents=True,exist_ok=True);adb=[str(Path.home()/'Android/Sdk/platform-tools/adb'),'-s',a.serial];pkg='com.algosculptor.emulator.gpulab';notes=[]
 def call(*args,timeout=30):return subprocess.run(adb+list(args),check=True,capture_output=True,timeout=timeout).stdout
 def dump(phase):
  for retry in range(3):
   try:
    remote=f'/sdcard/app-{phase}-{retry}.xml';result=call('shell','uiautomator','dump',remote)
    if b'dumped to:' not in result:raise ValueError('Incomplete XML dump')
    raw=call('exec-out','cat',remote);root=ET.fromstring(raw);(out/f'{phase}-{retry}.xml').write_bytes(raw)
    if any(n.get('text','')=="System UI isn't responding" for n in root.iter('node')):
     tapnode(next(n for n in root.iter('node') if n.get('text','').lower()=='wait'));notes.append('System UI startup ANR Wait');continue
    assert not any("isn't responding" in n.get('text','') for n in root.iter('node')),'Unexpected ANR'
    return root
   except (subprocess.SubprocessError,ET.ParseError,ValueError) as e:notes.append(f'{phase} retry:{e}')
  raise RuntimeError('Fresh UI unavailable:'+phase)
 def bounds(n):return tuple(map(int,re.findall(r'\d+',n.get('bounds'))))
 def tapnode(n):
  x,y,x2,y2=bounds(n);call('shell','input','tap',str((x+x2)//2),str((y+y2)//2))
 def matches(r,text):return [n for n in r.iter('node') if n.get('text','').lower()==text.lower()]
 def wait(text,phase,contains=False,seconds=30):
  end=time.monotonic()+seconds;i=0
  while time.monotonic()<end:
   r=dump(f'{phase}-{i}');i+=1
   if any((text.lower() in n.get('text','').lower() if contains else n.get('text','').lower()==text.lower()) for n in r.iter('node')):print('Verified',phase,flush=True);return r
  raise RuntimeError('Missing UI:'+text)
 def tap(text,phase):r=wait(text,phase);tapnode(matches(r,text)[-1]);return r
 def shot(phase):(out/f'{phase}.png').write_bytes(call('exec-out','screencap','-p'))
 def scroll(r):
  n=next(n for n in r.iter('node') if n.get('scrollable')=='true');x,y,x2,y2=bounds(n);call('shell','input','swipe',str((x+x2)//2),str(y2-220),str((x+x2)//2),str(y+180),'300')
 def visible(text,phase):
  for i in range(5):
   r=dump(phase+'-'+str(i))
   if matches(r,text):return r
   scroll(r)
  raise RuntimeError('Not reachable by scrolling:'+text)
 def choose(name):
  tap('Import homebrew','open-import-'+name)
  r=dump('picker-'+name)
  # Open the provider drawer, using only this tree's current control bounds.
  drawer=[n for n in r.iter('node') if n.get('content-desc','').lower() in ['show roots','open navigation drawer']]
  if drawer:tapnode(drawer[0]);r=dump('roots-'+name)
  for i in range(3):
   choices=matches(r,'QA Documents')
   if choices:tapnode(choices[-1]);break
   scroll(r);r=dump('roots-scroll-'+str(i))
  else:raise RuntimeError('QA provider not available')
  r=dump('documents-view-'+name)
  lists=[n for n in r.iter('node') if n.get('content-desc','')=='List view']
  if lists:tapnode(lists[0])
  for i in range(4):
   r=dump('documents-'+name+'-'+str(i));choices=matches(r,name)
   if choices:tapnode(choices[-1]);return
   scroll(r)
  raise RuntimeError('QA file not visible:'+name)
 def catalog(phase):
  raw=call('exec-out','run-as',pkg,'cat','databases/library.db');path=out/(phase+'.db');path.write_bytes(raw)
  with sqlite3.connect(path) as db:rows=db.execute('SELECT id,title,filename,bytes,mode,last_played,launches,result FROM titles').fetchall()
  return rows
 def home():call('shell','am','start','-W','-f','0x10008000','-n',pkg+'/.HomeActivity');return wait('Your library','home')
 def remove(title,phase):
  r=wait(title,phase);title_node=matches(r,title)[0];ty=bounds(title_node)[1]
  candidates=[n for n in r.iter('node') if n.get('text','')=='Details' and bounds(n)[1]>=ty]
  if not candidates:scroll(r);r=dump(phase+'-scrolled');candidates=matches(r,'Details')
  tapnode(candidates[0]);tap('Remove',phase+'-details');tap('Remove',phase+'-confirm');wait('Removed from library.',phase+'-removed')
 try:
  call('install','-r',str(a.apk),timeout=60);call('install','-r',str(a.provider_apk),timeout=60);call('logcat','-c');home();shot('library-initial')
  call('shell','settings','put','system','accelerometer_rotation','0');call('shell','settings','put','system','user_rotation','1')
  visible('Play Catch','landscape-library');shot('library-landscape');call('shell','settings','put','system','user_rotation','0');home()
  # A slow pipe gives real cancellation and rotation coverage, not a synthetic callback.
  choose('slow-hello.nro');tap('Cancel import','cancel-import');wait('Import cancelled.','cancelled')
  rows=catalog('after-cancel');assert len(rows)==1,rows
  assert not call('exec-out','run-as',pkg,'find','files/library','-name','.import-*').strip()
  tap('Dismiss','dismiss-cancel');choose('slow-ipc.nro');wait('Cancel import','import-before-rotation')
  call('shell','settings','put','system','accelerometer_rotation','0');call('shell','settings','put','system','user_rotation','1')
  wait('Added slow-ipc.nro','rotated-import',seconds=45)
  call('shell','settings','put','system','user_rotation','0');wait('Your library','portrait')
  rows=catalog('after-rotation');assert any(row[1]=='slow-ipc.nro' for row in rows),rows
  tap('Dismiss','dismiss-added');choose('hello.nro');wait('Added hello.nro','hello-added');shot('library-imported')
  tap('Dismiss','dismiss-hello');choose('duplicate.nro');wait('already in your library','duplicate-rejected',contains=True);assert len(catalog('after-duplicate'))==3
  tap('Dismiss','dismiss-duplicate');choose('invalid.nro');wait('missing NRO0','invalid-rejected',contains=True);assert len(catalog('after-invalid'))==3
  tap('Dismiss','dismiss-invalid');choose('module.nro');wait('module headers','module-rejected',contains=True);assert len(catalog('after-module'))==3
  tap('Dismiss','dismiss-module');choose('oversize.nro');wait('up to 32 MiB','oversize-rejected',contains=True);assert len(catalog('after-oversize'))==3
  assert not call('exec-out','run-as',pkg,'find','files/library','-name','.import-*').strip()
  tap('Dismiss','dismiss-oversize');choose('bad-opcode.nro');wait('Added bad-opcode.nro','bad-opcode-added')
  call('shell','am','force-stop',pkg);home();r=dump('search-fault');edit=next(n for n in r.iter('node') if n.get('class')=='android.widget.EditText');tapnode(edit);call('shell','input','text','bad-opcode');call('shell','input','keyevent','4')
  tap('Run homebrew','run-fault');wait('unsupported A64','explicit-guest-fault',contains=True);shot('program-fault');tap('Back to library','fault-back')
  remove('bad-opcode.nro','remove-fault');assert len(catalog('after-remove'))==3
  call('shell','am','force-stop',pkg);home()
  # Select hello through search so Run refers to the chosen uploaded file.
  r=dump('search-hello');edit=next(n for n in r.iter('node') if n.get('class')=='android.widget.EditText');tapnode(edit);call('shell','input','text','hello');call('shell','input','keyevent','4')
  wait('hello.nro','hello-filtered');tap('Run homebrew','run-hello');wait('Exited normally','hello-exited');wait('Hello from original Switch homebrew!','real-guest-output',contains=True);shot('program-output')
  call('shell','settings','put','system','user_rotation','1');visible('Copy output','landscape-program');shot('program-landscape');call('shell','settings','put','system','user_rotation','0');wait('Exited normally','portrait-program')
  tap('Back to library','program-back');call('shell','am','force-stop',pkg);home();rows=catalog('cold-catalog')
  assert any(r[1]=='hello.nro' and r[6]>=1 and r[7]=='Exited normally' for r in rows),rows
  # The private imported copy remains usable after provider removal.
  call('uninstall','com.algosculptor.emulator.qadocuments');home()
  r=dump('search-after-provider-removal');edit=next(n for n in r.iter('node') if n.get('class')=='android.widget.EditText');tapnode(edit);call('shell','input','text','hello');call('shell','input','keyevent','4');tap('Run homebrew','run-private-copy');wait('Exited normally','private-copy-exited');wait('Hello from original Switch homebrew!','private-copy-output',contains=True)
  tap('Back to library','private-copy-back');home()
  # Clear a retained search query before removing or navigating settings.
  r=dump('clear-query');edit=next(n for n in r.iter('node') if n.get('class')=='android.widget.EditText');tapnode(edit);call('shell','input','keyevent','123');call('shell','input','keyevent','--longpress','67');call('shell','input','keyevent','4')
  tap('Settings','settings');r=wait('Show frame and GPU details','stats-setting');tapnode(matches(r,'Show frame and GPU details')[0]);tap('Held control speed: Normal','speed');tap('Slow','slow-speed');wait('Held control speed: Slow','speed-applied');shot('settings')
  tap('Back to library','settings-back');call('shell','am','force-stop',pkg);home();tap('Settings','settings-reopened');wait('Held control speed: Slow','speed-persisted');r=wait('Show frame and GPU details','stats-persisted');assert matches(r,'Show frame and GPU details')[0].get('checked')=='true'
  tap('Back to library','settings-back-again');r=home();edit=next(n for n in r.iter('node') if n.get('class')=='android.widget.EditText');tapnode(edit);call('shell','input','text','Catch');call('shell','input','keyevent','4')
  tap('Play Catch','play-catch');wait('Presented','player-stats-enabled',contains=True);shot('player');tap('Back to library','player-back');home();rows=catalog('final-catalog');assert next(r for r in rows if r[0]=='builtin:catch')[6]>0
  result={'passed':True,'checks':['main library launcher','landscape library controls reachable','landscape program controls reachable','slow SAF import cancellation','import survives configuration change','real NRO validation','duplicate rejection','bad header rejection','module rejection','oversize limit and cleanup','search selects actual file','real guest output and exit','unsupported instruction is a visible fault','remove deletes only catalog/private copy','catalog/recents survive cold start','private copy works after provider removal','persistent settings','settings apply to player','built-in live player session tracking'],'notes':notes}
  tap('Settings','restore-settings');tap('Held control speed: Slow','restore-speed');tap('Normal','restore-normal');r=wait('Show frame and GPU details','restore-stats');tapnode(matches(r,'Show frame and GPU details')[0]);tap('Back to library','restored-defaults')
  assert not call('logcat','-b','crash','-d').strip()
  assert not re.search(rb'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab',call('logcat','-d'))
  (out/'verification.json').write_text(json.dumps(result,indent=2));print(json.dumps(result),flush=True)
 finally:
  try:call('shell','settings','put','system','user_rotation','0');call('shell','settings','put','system','accelerometer_rotation','1')
  except subprocess.SubprocessError:pass
  for name,args in [('logcat.txt',['logcat','-d']),('crash-buffer.txt',['logcat','-b','crash','-d']),('exit-info.txt',['shell','dumpsys','activity','exit-info',pkg])]:
   try:(out/name).write_bytes(call(*args))
   except subprocess.SubprocessError:pass
if __name__=='__main__':main()
