import base64,datetime,subprocess,time
from pathlib import Path
cfg='/workspace/codex/ssh_config';alias='codex-ada2build-native581-56302930'
log=Path('/workspace/codex/ada2build20261016-native-stage.log')
def note(s):
    with log.open('a') as f:f.write(datetime.datetime.now(datetime.timezone.utc).isoformat()+' '+s+'\n')
def run(args,timeout=60):
    return subprocess.run(args,capture_output=True,text=True,encoding='utf8',errors='replace',timeout=timeout)
def ps(s,timeout=60):
    encoded=base64.b64encode(s.encode('utf-16le')).decode()
    return run(['ssh','-F',cfg,'-o','ConnectTimeout=5','-o','ForwardAgent=no',alias,'powershell.exe','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand',encoded],timeout)
rc=1;note('START own56302930, pinned final Windows key required')
try:
    deadline=__import__('datetime').datetime(2026,10,17,0,15,tzinfo=__import__('datetime').timezone.utc).timestamp()
    while time.time()<deadline:
        result=ps('[Environment]::OSVersion.VersionString',15)
        if result.returncode==0 and '26100' in result.stdout:break
        note('NATIVE_PENDING_RC='+str(result.returncode));time.sleep(20)
    else:raise RuntimeError('native Windows deadline exceeded')
    result=ps("$ErrorActionPreference='Stop'; foreach($p in 'C:\\ProgramData\\KayfabeSweep','C:\\ProgramData\\KayfabeSweep\\cuda','C:\\ProgramData\\KayfabeSweep\\caps') { New-Item -Type Directory -Force $p | Out-Null }; 'NATIVE_DIRECTORIES_READY=1'")
    if result.returncode:raise RuntimeError('mkdir failed')
    sources=[('/workspace/codex/windows-probes/kf_dxprobe.exe','kf_dxprobe.exe'),('/workspace/codex/ada2build20261016_native581_oracle.ps1','native_oracle.ps1')]
    sources += [('/workspace/codex/blackwell-hw/tools/windows-capture/install/install.ps1','install.ps1'),('/workspace/codex/turinghold20261016_native_desktop.ps1','native_desktop.ps1'),('/workspace/codex/ada2build20261016_native581_hold.ps1','hold.ps1')]
    sources += [(str(p),'caps/'+p.name) for p in Path('/workspace/codex/bwbatch-tracer').iterdir() if p.name in ('kfwc.sys','kfwc_collect.exe','SHA256SUMS')]
    sources += [('/workspace/codex/ada2build20261016_set_gsp8151.ps1','set_gsp8151.ps1')]
    sources += [(f'/workspace/codex/windows-probes/cuda/{n}.exe',f'cuda/{n}.exe') for n in ('cup2','cup3','cup8')]
    for source,target in sources:
        result=run(['scp','-q','-F',cfg,'-o','ForwardAgent=no',source,alias+':C:/ProgramData/KayfabeSweep/'+target],120)
        if result.returncode:raise RuntimeError('stage failed: '+target)
        note('STAGED '+target)
    result=ps("Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,ConfigManagerErrorCode,Status | ConvertTo-Json -Compress; if(Test-Path C:\\ProgramData\\VastWindows\\state.json) { $s=Get-Content -Raw C:\\ProgramData\\VastWindows\\state.json | ConvertFrom-Json; \"SETUP_RUNS=$($s.runs) SETUP_FAILURES=$($s.failures)\" }; 'NATIVE_STAGE_EXIT=0'",120)
    Path('/workspace/codex/ada2build20261016-native-preflight.txt').write_text(result.stdout+result.stderr)
    if result.returncode:raise RuntimeError('staged preflight failed')
    rc=0
except Exception as e:note('ERROR '+type(e).__name__+': '+str(e))
finally:note('EXIT rc='+str(rc))
raise SystemExit(rc)
