#!/usr/bin/env python3
"""Exercise the real verified APK -> Android permission/installer -> app upgrade flow."""
import argparse,json,re,sqlite3,subprocess,time,xml.etree.ElementTree as ET
from pathlib import Path

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=Path,required=True);p.add_argument('--resume-installed',action='store_true');a=p.parse_args();out=a.output;out.mkdir(parents=True,exist_ok=True)
 adb=[str(Path.home()/'Android/Sdk/platform-tools/adb'),'-s','emulator-5580'];pkg='com.algosculptor.emulator.gpulab';checks=[];notes=[]
 def call(*args,timeout=30):return subprocess.check_output(adb+list(args),stderr=subprocess.PIPE,timeout=timeout)
 def bounds(n):return list(map(int,re.findall(r'\d+',n.get('bounds'))))
 def tapnode(n):
  x,y,x2,y2=bounds(n);call('shell','input','touchscreen','swipe',str((x+x2)//2),str((y+y2)//2),str((x+x2)//2),str((y+y2)//2),'80')
 def ui(phase):
  for i in range(3):
   remote=f'/sdcard/updater-{phase}-{i}.xml';r=call('shell','uiautomator','dump',remote)
   if b'dumped to:' not in r:continue
   raw=call('exec-out','cat',remote);root=ET.fromstring(raw);(out/f'{phase}-{i}.xml').write_bytes(raw)
   if any(n.get('text')=="System UI isn't responding"for n in root.iter('node')):
    tapnode(next(n for n in root.iter('node')if n.get('text','').lower()=='wait'));notes.append('Unrelated System UI startup Wait');continue
   assert not any("isn't responding"in n.get('text','')for n in root.iter('node'));return root
  raise RuntimeError('No fresh XML '+phase)
 def wait(text,phase,seconds=35):
  end=time.monotonic()+seconds;i=0
  while time.monotonic()<end:
   root=ui(phase+'-'+str(i));i+=1
   matches=[n for n in root.iter('node')if n.get('text','').lower()==text.lower()]
   if matches:return root,matches[-1]
   scroll=next((n for n in root.iter('node')if n.get('scrollable')=='true'),None)
   if scroll:
    x,y,x2,y2=bounds(scroll);call('shell','input','swipe',str((x+x2)//2),str(y2-100),str((x+x2)//2),str(y+100),'250')
  raise RuntimeError('Missing '+text)
 def tap(text,phase):r,n=wait(text,phase);tapnode(n);return r
 def shot(phase):(out/f'{phase}.png').write_bytes(call('exec-out','screencap','-p'))
 def rows(phase):
  p=out/(phase+'.db');p.write_bytes(call('exec-out','run-as',pkg,'cat','databases/library.db'))
  with sqlite3.connect(p)as db:return db.execute('select * from titles order by id').fetchall()
 def verified(name):checks.append(name);(out/'progress.json').write_text(json.dumps({'checks':checks},indent=2));print('Verified',name,flush=True)
 def ready():return call('exec-out','run-as',pkg,'cat','files/updates/ready.json')
 try:
  if a.resume_installed:
   checks[:]=json.loads((out/'progress.json').read_text())['checks'];assert len(checks)==7
   assert 'versionName=0.27.2-dev'in call('shell','dumpsys','package',pkg).decode()
   with sqlite3.connect(out/'before-update.db')as db:before=db.execute('select * from titles order by id').fetchall()
   assert rows('after-update')==before
   notes.append('Resumed after recorded successful permission/cancel/install and exact library assertions; fixed exec-out remote-status test to use shell test -f')
  else:
   initial=ui('initial-window')
   if any(n.get('text')=='App installed.'for n in initial.iter('node')):
    done=next(n for n in initial.iter('node')if n.get('text','').lower()=='done');tapnode(done);notes.append('Dismissed prior QA installer success window')
   assert 'versionName=0.27.1-dev'in call('shell','dumpsys','package',pkg).decode()
   call('shell','appops','set',pkg,'REQUEST_INSTALL_PACKAGES','default');call('logcat','-c');call('shell','am','force-stop',pkg);call('shell','am','start','-W','-f','0x10008000','-n',pkg+'/.HomeActivity');wait('Install app update','ready-home');shot('ready-home');before=rows('before-update');saved=ready();verified('Ready update restored on cold library launch')
   tap('Install app update','open-updates');wait('Install update','ready-button');shot('ready-updates');verified('Home banner opens private update screen')
   for label,phase in [('Check automatically','enable-auto'),('Download on Wi-Fi','enable-download')]:
    _,n=wait(label,phase)
    if n.get('checked')!='true':tapnode(n)
   verified('Automatic check and Wi-Fi preferences editable')
   tap('Install update','permission-launch');r,n=wait('Allow from this source','unknown-source')
   toggle=None
   for attempt in range(4):
    toggle=next((v for v in r.iter('node')if v.get('checkable')=='true'),None)
    if toggle is not None:break
    r=ui('unknown-source-toggle-'+str(attempt))
   assert toggle is not None
   if toggle.get('checked')!='true':tapnode(toggle)
   shot('unknown-source-allowed');call('shell','input','keyevent','4');verified('Explicit Android install-source permission handled')
   r,_=wait('Update','installer-confirmation');assert any(n.get('text')=='Do you want to update this app?'for n in r.iter('node'));n=next(n for n in r.iter('node')if n.get('text','').lower()=='cancel');shot('installer-confirmation');tapnode(n);wait('Install update','installer-cancelled');assert ready()==saved;verified('Cancelling Android installer retains verified APK')
   tap('Install update','install-retry');r=ui('installer-retry')
   candidates=[v for v in r.iter('node')if v.get('text','').lower()in('update','install')]
   if not candidates:
    r,n=wait('Update','installer-update');candidates=[n]
   tapnode(candidates[-1]);verified('Android confirmation installs through granted content URI')
   end=time.monotonic()+60
   while time.monotonic()<end:
    r=call('shell','dumpsys','package',pkg).decode()
    if 'versionName=0.27.2-dev'in r:break
    time.sleep(.2)
   else:raise RuntimeError('Installed package did not advance')
   r,n=wait('Open','installer-success');assert any(v.get('text')=='App installed.'for v in r.iter('node'));tapnode(n);wait('Your library','upgraded-library');shot('upgraded-library');assert rows('after-update')==before;verified('Real self-update advances version and preserves library rows')
  end=time.monotonic()+15
  while time.monotonic()<end:
   r=subprocess.run(adb+['shell','run-as',pkg,'test','-f','files/updates/ready.apk'],capture_output=True)
   if r.returncode!=0:break
   time.sleep(.1)
  else:raise RuntimeError('Installed APK cache not removed')
  verified('Installed update cache removed after app restart')
  tap('Settings','updated-settings');tap('App updates','updated-updates');wait('Installed 0.27.2-dev','updated-version');shot('updated-version');verified('Updated app reports actual installed version')
  call('shell','settings','put','system','accelerometer_rotation','0');call('shell','settings','put','system','user_rotation','1');wait('App updates','updates-landscape');shot('updates-landscape');call('shell','settings','put','system','user_rotation','0');verified('Updates screen recreates in landscape and portrait')
  tap('Connect GitHub','connect-dialog');r,n=wait('Connect private updates','connect-dialog-open');shot('connect-dialog');edit=next(v for v in r.iter('node')if v.get('class')=='android.widget.EditText');assert edit.get('password')=='true';tap('Connect','reject-empty');wait('Connect private updates','invalid-token-retained');tap('Cancel','cancel-connect');verified('Private token entry is masked and rejects empty access')
  assert not call('logcat','-b','crash','-d').strip();assert not re.search(rb'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab',call('logcat','-d'))
  result={'passed':True,'checks':checks,'notes':notes,'fromVersion':'0.27.1-dev','installedVersion':'0.27.2-dev','libraryPreserved':True};(out/'verification.json').write_text(json.dumps(result,indent=2));print(json.dumps(result),flush=True)
 finally:
  for name,args in [('logcat.txt',['logcat','-d']),('crash-buffer.txt',['logcat','-b','crash','-d']),('exit-info.txt',['shell','dumpsys','activity','exit-info',pkg])]:
   try:(out/name).write_bytes(call(*args))
   except subprocess.SubprocessError:pass
  try:call('shell','settings','put','system','user_rotation','0');call('shell','settings','put','system','accelerometer_rotation','1')
  except subprocess.SubprocessError:pass
if __name__=='__main__':main()
