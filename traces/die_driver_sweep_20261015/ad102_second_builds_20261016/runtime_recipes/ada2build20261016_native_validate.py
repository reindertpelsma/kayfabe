import base64,datetime,json,subprocess
from pathlib import Path
root=Path('/workspace/codex');receipt=json.loads((root/'ada2build-20261016-owned.json').read_text());assert receipt['id']==56302930

log=root/'ada2build20261016-native-validate.log'
def note(s):
 with log.open('a') as f:f.write(datetime.datetime.now(datetime.timezone.utc).isoformat()+' '+s+'\n')
def ps(s,timeout):
 enc=base64.b64encode(s.encode('utf-16le')).decode()
 return subprocess.run(['ssh','-F',str(root/'ssh_config'),'-o','ForwardAgent=no','codex-ada2build-native581-56302930','powershell.exe','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand',enc],capture_output=True,text=True,encoding='utf8',errors='replace',timeout=timeout)
rc=1;note('START own56302930 H_AD102_581_REF falsifier=native58151 Code/CUDA/D3D failure or System4101 during420s hold; no reimage on failure')
try:
 r=ps("if((Get-Service KayfabeWinCapture).Status -ne 'Stopped') { throw 'Measurement module active' }; if(@(Get-ChildItem Env: | Where-Object Name -like 'KF3_*').Count) { throw 'Unexpected KF3 native environment' }; 'NATIVE_KF3_ENV={} MODULE_STOPPED=1'",60)
 if r.returncode:raise RuntimeError('native quiescent precondition failed')
 for script,limit,label in [('native_oracle.ps1',180,'oracle-before'),('hold.ps1',660,'hold'),('native_oracle.ps1',180,'oracle-after')]:
  note(label+'_START')
  r=ps('& C:\\ProgramData\\KayfabeSweep\\'+script,limit)
  (root/('ada2build20261016-native-'+label+'.txt')).write_text(r.stdout+r.stderr+'\nREMOTE_EXIT_RC='+str(r.returncode)+'\n')
  note(label+'_EXIT='+str(r.returncode))
  if r.returncode:raise RuntimeError('native '+label+' failed; no automatic reimage')
 note('NATIVE_REFERENCE_PASS_NO_REIMAGE_YET=1');rc=0
except Exception as e:note('ERROR '+type(e).__name__+': '+str(e))
finally:note('EXIT rc='+str(rc))
raise SystemExit(rc)
