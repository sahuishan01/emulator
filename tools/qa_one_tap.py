#!/usr/bin/env python3
"""Real one-tap visible patterns, Downloads JSON, cancellation and restart QA."""
import argparse,json,re,sqlite3,struct,subprocess,time,zlib,xml.etree.ElementTree as ET
from pathlib import Path
from qa_pattern_pixels import expected_pixel

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apk',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--previous-workload',type=int,default=29);p.add_argument('--version',default='0.27.0-dev');p.add_argument('--workload',type=int,default=30);p.add_argument('--pattern-count',type=int,default=55);p.add_argument('--gpu-tests',type=int,default=71);p.add_argument('--native-workload',type=int,default=15);p.add_argument('--reference-prefix',default='reports/v0.27-host-');p.add_argument('--resume-lifecycle',action='store_true',help='Finish recovery QA from recorded completed captures and lifecycle UI evidence');a=p.parse_args()
 out=a.output;out.mkdir(parents=True,exist_ok=True);pkg='com.algosculptor.emulator.gpulab';adb=[str(Path.home()/'Android/Sdk/platform-tools/adb'),'-s','emulator-5580'];notes=[];completed_reports=[]
 def call(*args,timeout=35):return subprocess.check_output(adb+list(args),timeout=timeout,stderr=subprocess.PIPE)
 def cache(name='last-report.json'):return json.loads(call('exec-out','run-as',pkg,'cat','files/'+name))
 def download_names():
  r=subprocess.run(adb+['shell','ls /sdcard/Download/Emulator 2>/dev/null'],capture_output=True)
  return {n for n in r.stdout.decode().splitlines() if n.startswith('emulator-gpu-report-') and n.endswith('.json')}
 def bounds(n):return tuple(map(int,re.findall(r'\d+',n.get('bounds'))))
 def tapnode(n):
  x,y,x2,y2=bounds(n);call('shell','input','tap',str((x+x2)//2),str((y+y2)//2))
 def ui(phase):
  for retry in range(3):
   remote=f'/sdcard/one-tap-{phase}-{retry}.xml';r=call('shell','uiautomator','dump',remote)
   if b'dumped to:' not in r:continue
   raw=call('exec-out','cat',remote);root=ET.fromstring(raw);(out/f'{phase}-{retry}.xml').write_bytes(raw)
   if any(n.get('text')=="System UI isn't responding" for n in root.iter('node')):
    tapnode(next(n for n in root.iter('node') if n.get('text','').lower()=='wait'));notes.append('System UI startup Wait');continue
   assert not any("isn't responding" in n.get('text','') for n in root.iter('node'))
   return root
  raise RuntimeError('Fresh XML unavailable:'+phase)
 def tap(text,phase):
  root=ui(phase)
  for attempt in range(3):
   matches=[n for n in root.iter('node') if n.get('text','').lower()==text.lower()]
   if matches:tapnode(matches[0]);return
   scroll=next((n for n in root.iter('node') if n.get('scrollable')=='true'),None)
   if scroll is None:break
   x,y,x2,y2=bounds(scroll);call('shell','input','swipe',str((x+x2)//2),str(y2-100),str((x+x2)//2),str(y+100),'250');root=ui(phase+'-scroll'+str(attempt))
  raise RuntimeError('Missing control:'+text)
 def shot(name):(out/f'{name}.png').write_bytes(call('exec-out','screencap','-p'))
 def wait_scene(excluded,seconds=30):
  end=time.monotonic()+seconds
  while time.monotonic()<end:
   try:
    r=cache('last-texture-report.json')
    if r.get('appVersion')==a.version and r.get('batchRunId') not in excluded and r.get('passed'):return r
   except (subprocess.SubprocessError,json.JSONDecodeError):pass
   time.sleep(.1)
  raise RuntimeError('New visible pattern did not complete')
 def verify_report(raw):
  r=json.loads(raw);assert r['appVersion']==a.version and r['workloadVersion']==a.workload and r['schemaVersion']==4 and r['allPassed']
  assert {k:len(r[k]['tests']) for k in ['engine','nv','gpuCore','shader','gpu']}=={'engine':18,'nv':50,'gpuCore':76,'shader':25,'gpu':a.gpu_tests}
  assert r['gpu']['workloadVersion']==a.native_workload and r['gpu']['validationErrors']==0
  for key,name in [('engine','engine'),('nv','nv'),('gpuCore','gpu-core'),('shader','shader')]:
   reference=json.load(open(a.reference_prefix+name+'.json'));actual=r[key].copy();reference.pop('hostArchitecture',None);actual.pop('hostArchitecture',None);assert actual==reference,key
  b=r['patternBatch'];assert b['status']=='complete' and b['totalPatterns']==b['passedPatterns']==len(b['patterns'])==a.pattern_count and b['allPassed']
  assert [p['variant'] for p in b['patterns']]==list(range(a.pattern_count))
  for pattern in b['patterns']:
   assert pattern['batchRunId']==b['runId'] and pattern['passed'] and pattern['surfaceUpdated'] and pattern['pixelsVerified']
   assert pattern['completedFrames']==pattern['presentedFrames']==1 and pattern['validationErrors']==0 and pattern['error']==''
  return r
 def verify_export(phase,excluded=(),capture_bounds=None):
  end=time.monotonic()+420;seen=set();active_id=None;capture_runs=0;excluded=set(excluded)
  while time.monotonic()<end:
   if capture_bounds:
    try:
     s=cache('last-texture-report.json');v=s.get('variant');candidate=s.get('batchRunId')
     if candidate and candidate not in excluded:active_id=candidate
     if candidate and candidate not in excluded and s.get('passed') and v not in seen:
      if verify_pixels(s,capture_bounds,phase+'-pattern-'+str(v)):seen.add(v)
    except (subprocess.SubprocessError,json.JSONDecodeError):pass
   try:
    raw=call('exec-out','run-as',pkg,'cat','files/last-report.json');r=json.loads(raw)
    if r.get('appVersion')!=a.version or r.get('patternBatch',{}).get('runId') in excluded or 'automaticExport' not in r:time.sleep(.1);continue
    filename=r['automaticExport']['filename'];data=call('exec-out','cat','/sdcard/Download/Emulator/'+filename)
    if data!=raw:time.sleep(.1);continue
    metadata=call('shell','content','query','--uri',r['automaticExport']['uri'],'--projection','mime_type:is_pending:relative_path').decode()
    if 'is_pending=0' not in metadata:time.sleep(.1);continue
    assert 'mime_type=application/json' in metadata and 'relative_path=Download/Emulator/' in metadata
    r=verify_report(raw);assert not active_id or r['patternBatch']['runId']==active_id
    completed_reports.append(r)
    (out/(phase+f'-run-{len(completed_reports)}.json')).write_bytes(raw)
    if capture_bounds and seen!=set(range(a.pattern_count)):
     capture_runs+=1
     assert capture_runs<=2,('Missing displayed captures after bounded retries',seen)
     excluded.add(r['patternBatch']['runId']);active_id=None
     notes.append(f"Capture crossed a pattern transition; collect remaining {sorted(set(range(a.pattern_count))-seen)} in a fresh run")
     tap('Run again',phase+f'-capture-retry-{capture_runs}')
     continue
    (out/(phase+'.json')).write_bytes(raw)
    print('Verified',phase,f'{a.pattern_count} visible patterns +{sum(len(r[k]["tests"]) for k in ["engine","nv","gpuCore","shader","gpu"])} checks; exact published Downloads JSON',flush=True);return r
   except (subprocess.SubprocessError,json.JSONDecodeError):pass
   time.sleep(.1)
  raise RuntimeError('Automatic export did not complete:'+phase)
 def verify_pixels(scene,box,phase):
  x,y,x2,y2=box;w,h=x2-x,y2-y;dw=w;dh=max(1,dw//2)
  if dh>h:dh=h;dw=max(1,dh*2)
  left=x+(w-dw)//2;top=y+(h-dh)//2
  raw=call('exec-out','screencap');sw,sh,fmt=struct.unpack_from('<3I',raw);header=len(raw)-sw*sh*4;assert fmt in(1,2) and header in(12,16)
  after=cache('last-texture-report.json')
  if (after.get('batchRunId'),after.get('variant'),after.get('capturedAtEpochMs'))!=(scene.get('batchRunId'),scene.get('variant'),scene.get('capturedAtEpochMs')):return False
  variant=scene['variant']
  for py in range(8):
   for px in range(16):
    expected=expected_pixel(variant,px,py)
    xx=left+(2*px+1)*dw//32;yy=top+(2*py+1)*dh//16;at=header+(yy*sw+xx)*4
    assert tuple(raw[at:at+4])==expected,(phase,px,py,tuple(raw[at:at+4]),expected)
  def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
  rgba=raw[header:];scan=b''.join(b'\0'+rgba[y*sw*4:(y+1)*sw*4] for y in range(sh))
  png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',sw,sh,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(scan,1))+chunk(b'IEND',b'')
  (out/f'{phase}.png').write_bytes(png)
  print('Verified',phase,'128 displayed pixel centers',flush=True);return True
 try:
  assert call('shell','getprop','sys.boot_completed').strip()==b'1'
  if a.resume_lifecycle:
   previous=json.loads((out/'before-upgrade-report.json').read_bytes());assert previous['workloadVersion']==a.previous_workload
   first=verify_report((out/'first-automatic-report.json').read_bytes())
   completed_reports[:]=[verify_report(f.read_bytes()) for f in sorted(out.glob('first-automatic-report-run-*.json'))]
   assert completed_reports and len({r['patternBatch']['runId'] for r in completed_reports})==len(completed_reports)
   for name in ['cancelled-0.xml','share-chooser-0.xml','completed-landscape-0.xml','resumed-batch-0.xml']:
    assert (out/name).is_file(),name
   assert {int(f.stem.rsplit('-',1)[-1]) for f in out.glob('first-automatic-report-pattern-*.png')}==set(range(a.pattern_count))
   current=verify_report(call('exec-out','run-as',pkg,'cat','files/last-report.json'))
   assert current['patternBatch']['runId'] not in {r['patternBatch']['runId'] for r in completed_reports}
   known={r['automaticExport']['filename'] for r in [*completed_reports,current]}
   initial=download_names()-known
   for name in initial:
    assert json.loads(call('exec-out','cat','/sdcard/Download/Emulator/'+name)).get('appVersion')!=a.version
   interrupted=rotated=None
   notes.append('Runner interrupted after recorded completed captures and lifecycle UI; resumed final published report and recovery checks')
  else:
   call('shell','am','force-stop',pkg)
   old=call('exec-out','run-as',pkg,'cat','files/last-report.json');previous=json.loads(old);assert previous['workloadVersion']==a.previous_workload;(out/'before-upgrade-report.json').write_bytes(old)
   def rows(phase):
    p=out/(phase+'.db');p.write_bytes(call('exec-out','run-as',pkg,'cat','databases/library.db'))
    with sqlite3.connect(p) as c:return c.execute('select * from titles order by id').fetchall()
   before=rows('before-upgrade-library');call('install','-r',str(a.apk),timeout=60);call('logcat','-c')
   call('shell','am','start','-W','-n',pkg+'/.HomeActivity');assert rows('after-upgrade-library')==before
   assert call('exec-out','run-as',pkg,'cat','files/last-report.json')==old
   initial=download_names();tap('Test all & export JSON','one-tap-launch');tap('Cancel test','cancel-running')
   r=ui('cancelled');assert any('Test cancelled' in n.get('text','') for n in r.iter('node'));assert download_names()==initial
   box=bounds(next(n for n in r.iter('node') if n.get('content-desc')=='Automatic test patterns'))
   first_attempt=cache('last-texture-report.json').get('batchRunId')
   tap('Run again','restart-after-cancel');first=verify_export('first-automatic-report',(first_attempt,previous.get('patternBatch',{}).get('runId'),None),box)
   assert download_names()-initial=={r['automaticExport']['filename'] for r in completed_reports};shot('automatic-result')
   tap('Share JSON','share-result');root=ui('share-chooser');shot('share-chooser');call('shell','input','keyevent','4');ui('share-return')
   assert download_names()-initial=={r['automaticExport']['filename'] for r in completed_reports}
   call('shell','settings','put','system','accelerometer_rotation','0');call('shell','settings','put','system','user_rotation','1');ui('completed-landscape');shot('completed-landscape')
   call('shell','settings','put','system','user_rotation','0');ui('completed-portrait');assert cache()['automaticExport']==first['automaticExport']
   # Interrupt a second attempt during rotation/background; only its fresh restart may publish.
   tap('Run again','lifecycle-start');interrupted=wait_scene({first['patternBatch']['runId']})['batchRunId']
   call('shell','settings','put','system','user_rotation','1');rotated=wait_scene({first['patternBatch']['runId'],interrupted})['batchRunId']
   call('shell','input','keyevent','3');time.sleep(.5);assert download_names()-initial=={r['automaticExport']['filename'] for r in completed_reports}
   call('shell','am','start','-W','-a','android.intent.action.MAIN','-c','android.intent.category.LAUNCHER','-f','0x10200000','-n',pkg+'/.HomeActivity')
   root=ui('resumed-batch')
   if any(n.get('text')=='Your library' for n in root.iter('node')):call('shell','input','keyevent','4')
  second=verify_export('restarted-automatic-report',(first['patternBatch']['runId'],interrupted,rotated))
  assert download_names()-initial=={r['automaticExport']['filename'] for r in completed_reports}
  call('shell','settings','put','system','user_rotation','0');ui('final-portrait')
  # The complete automatic report stays byte-exact when opened in manual Diagnostics.
  expected=(out/'restarted-automatic-report.json').read_bytes();call('shell','am','force-stop',pkg);call('shell','am','start','-W','-n',pkg+'/.HomeActivity');tap('Settings','cold-settings');tap('Diagnostics','cold-diagnostics')
  root=ui('cold-restored');assert any(f'All {sum(len(second[k]["tests"]) for k in ("engine","nv","gpuCore","shader","gpu"))} checks passed' in n.get('text','') and f'Patterns: {a.pattern_count}/{a.pattern_count}' in n.get('text','') for n in root.iter('node'))
  assert call('exec-out','run-as',pkg,'cat','files/last-report.json')==expected;shot('cold-restored')
  assert not call('logcat','-b','crash','-d').strip();assert not re.search(rb'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab',call('logcat','-d'))
  result={'passed':True,'checks':['one home button launches full workflow','upgrade preserves catalog/cache','cancel publishes no new JSON','all 55 automatically displayed patterns','7040 displayed pixel centers match formulas','all 240 diagnostics match host results','Downloads JSON byte-exact with cache','published application/json entry with no picker','Share opens chooser without another export','completed rotation preserves export','rotation restarts unfinished run','background restart excludes abandoned attempts','exactly one file per completed run','cold Diagnostics restores complete 55-pattern report'],'notes':notes,'completedRuns':len(completed_reports),'displaySamples':a.pattern_count*128}
  (out/'verification.json').write_text(json.dumps(result,indent=2));print(json.dumps(result),flush=True)
 finally:
  try:call('shell','settings','put','system','user_rotation','0');call('shell','settings','put','system','accelerometer_rotation','1')
  except subprocess.SubprocessError:pass
  for name,args in [('logcat.txt',['logcat','-d']),('crash-buffer.txt',['logcat','-b','crash','-d']),('exit-info.txt',['shell','dumpsys','activity','exit-info',pkg])]:
   try:(out/name).write_bytes(call(*args))
   except subprocess.SubprocessError:pass
if __name__=='__main__':main()
