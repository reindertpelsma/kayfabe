import base64,datetime,json,subprocess,time
from pathlib import Path
root=Path('/workspace/codex');prefix='ada2build20261016';cfg=root/'ssh_config'
receipt=json.loads((root/'ada2build-20261016-owned.json').read_text());assert receipt['id']==56302930
log=root/(prefix+'-native-orchestrate.log')
def note(s):
 with log.open('a') as f:f.write(datetime.datetime.now(datetime.timezone.utc).isoformat()+' '+s+'\n')
def ps(s,timeout=30):
 enc=base64.b64encode(s.encode('utf-16le')).decode()
 return subprocess.run(['ssh','-F',str(cfg),'-o','ForwardAgent=no','-o','ConnectTimeout=5','codex-ada2build-native581-56302930','powershell.exe','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand',enc],capture_output=True,text=True,encoding='utf8',errors='replace',timeout=timeout)
def save(label,r):
 (root/(prefix+'-native-'+label+'.txt')).write_text(r.stdout+r.stderr+'\nREMOTE_EXIT_RC='+str(r.returncode)+'\n')
 note(label+'_EXIT='+str(r.returncode))
rc=1;note('START own56302930 native Windows58151 reference; no automatic disk reimage')
try:
 deadline=datetime.datetime(2026,10,17,0,20,tzinfo=datetime.timezone.utc).timestamp()
 while time.time()<deadline:
  p=root/(prefix+'-native-stage.log');s=p.read_text() if p.exists() else ''
  if 'EXIT rc=0' in s:break
  if 'EXIT rc=1' in s:raise RuntimeError('stage failed; inspect before mutation')
  time.sleep(15)
 else:raise RuntimeError('native stage deadline exceeded')
 note('STAGE_EXIT0=1')
 while time.time()<deadline:
  try:r=ps("$s=Get-Content -Raw C:\\ProgramData\\VastWindows\\state.json | ConvertFrom-Json; $g=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\\VEN_10DE&DEV_2684*'); if($s.complete -and $g.Count -eq 1 -and $g[0].DriverVersion -eq '32.0.15.8151' -and $g[0].ConfigManagerErrorCode -eq 0) { 'SETUP_AD102_58151_COMPLETE=1' } else { 'WAIT_SETUP complete='+$s.complete+' failures='+$s.failures+' driver='+($g.DriverVersion -join ',') }")
  except subprocess.TimeoutExpired:note('SETUP_POLL_TIMEOUT');time.sleep(15);continue
  if r.returncode==0 and 'SETUP_AD102_58151_COMPLETE=1' in r.stdout:break
  note('SETUP_WAIT '+r.stdout.strip()[:200]);time.sleep(20)
 else:raise RuntimeError('actual58151/PCI2684 setup deadline exceeded')
 note('PINNED_NATIVE58151_SETUP_COMPLETE=1')
 r=ps("$ErrorActionPreference='Stop'; & C:\\ProgramData\\KayfabeSweep\\set_gsp8151.ps1; & C:\\ProgramData\\KayfabeSweep\\install.ps1 -Action Preflight; & C:\\ProgramData\\KayfabeSweep\\install.ps1 -Action EnableTestSigning; & C:\\ProgramData\\KayfabeSweep\\native_desktop.ps1; 'NATIVE_TESTSIGN_DESKTOP_REBOOT_QUEUED=1'",90)
 (root/(prefix+'-native-desktop-reboot-PRIVATE.txt')).write_text(r.stdout+r.stderr);(root/(prefix+'-native-desktop-reboot-PRIVATE.txt')).chmod(0o600);note('DESKTOP_REBOOT_PROGRAM_RC='+str(r.returncode))
 if r.returncode:note('DESKTOP_REBOOT_OUTCOME_UNCERTAIN; read-only verification follows, no program replay')
 time.sleep(25)
 for _ in range(30):
  try:r=ps("$b=(& bcdedit.exe /enum '{current}') -join \"`n\"; if($b -match 'testsigning\\s+Yes' -and @(Get-Process dwm -ErrorAction SilentlyContinue).Count -and @(Get-Process explorer -ErrorAction SilentlyContinue).Count) { 'NATIVE_DESKTOP_TESTSIGN_READY=1' }")
  except subprocess.TimeoutExpired:time.sleep(10);continue
  if r.returncode==0 and 'NATIVE_DESKTOP_TESTSIGN_READY=1' in r.stdout:break
  time.sleep(10)
 else:raise RuntimeError('own desktop/testsign reboot not verified')
 note('NATIVE_DESKTOP_TESTSIGN_READY=1');r=ps("& nvidia-smi.exe -q | Select-String 'GSP Firmware Version'; Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,ConfigManagerErrorCode | ConvertTo-Json -Compress",60);save('gsp-and-driver',r)
 # Public headers do not name caps word0x00c; no unprivileged RM verb exposes
 # this physical page. Read only the module's existing fixed caps aperture.
 r=ps("$ErrorActionPreference='Stop'; Set-Location C:\\ProgramData\\KayfabeSweep; if((Get-FileHash caps\\kfwc.sys).Hash.ToLower() -ne '1b0a549b7df5c9472dc6842eaf80e149fe7ec02a720fb9d874bade987413f1c6') { throw 'Unsigned tracer hash mismatch' }; & .\\install.ps1 -Action Install -Build C:\\ProgramData\\KayfabeSweep\\caps; & .\\caps\\kfwc_collect.exe --display-caps; $c=$LASTEXITCODE; & .\\install.ps1 -Action Stop; if((Get-Service KayfabeWinCapture).Status -ne 'Stopped') { throw 'Module not stopped' }; if($c) { throw 'Native fixed-aperture caps query failed' }; 'NATIVE_CAPS58151_EXIT=0'",120)
 save('caps58151',r)
 if r.returncode:raise RuntimeError('native caps measurement failed; inspect module state before workload')
 for script,limit,label in [('native_oracle.ps1',180,'oracle-before'),('hold.ps1',660,'hold'),('native_oracle.ps1',180,'oracle-after')]:
  if label=='hold' and time.time()+480>datetime.datetime(2026,10,17,0,30,tzinfo=datetime.timezone.utc).timestamp():note('NATIVE_HOLD_UNRUN=remaining_time');continue
  r=ps('& C:\\ProgramData\\KayfabeSweep\\'+script,limit);save(label,r)
  if r.returncode:raise RuntimeError('native '+label+' failed; no automatic Linux reimage')
 note('NATIVE_REFERENCE_PROGRAMS_PASS_NO_LINUX_RETURN=1');rc=0
except Exception as e:note('ERROR '+type(e).__name__+': '+str(e))
finally:note('EXIT rc='+str(rc))
raise SystemExit(rc)
