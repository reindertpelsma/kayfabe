import base64,json,subprocess,sys
w='/var/lib/kf-windows-20261005'; n=sys.argv[1]; helper=w+'/boundary-tools/qmp.py'; sock=n+'/qga.sock'
def ps(s):
 r=subprocess.run(['python3',helper,sock,'qga-exec','powershell.exe','-NoProfile','-Command',s],capture_output=True,text=True,timeout=60)
 print(r.stdout, r.stderr);
 if r.returncode: raise RuntimeError("guest command failed")
 return r.returncode
b=base64.b64encode(open('/tmp/kf_glgears_normal.exe','rb').read()).decode()
ps("New-Item -ItemType Directory -Force C:\\kf | Out-Null; Stop-Process -Name kf_glgears_stage -Force -ErrorAction SilentlyContinue; $h=[IO.File]::Create('C:\\kf\\kf_glgears_stage.exe');$h.Close()")
for i in range(0,len(b),24000):
 ps("$b=[Convert]::FromBase64String('"+b[i:i+24000]+"');$h=[IO.File]::Open('C:\\kf\\kf_glgears_stage.exe','Append');$h.Write($b,0,$b.Length);$h.Close()")
s=r"""& C:\kf\kf_glgears_stage.exe --seconds 45 > C:\kf\stage-out.txt 2> C:\kf\stage-err.txt; Add-Content C:\kf\stage-out.txt ('EXIT '+$LASTEXITCODE)"""
b=base64.b64encode(s.encode('utf-8')).decode()
ps("[IO.File]::WriteAllBytes('C:\\kf\\stage.ps1',[Convert]::FromBase64String('"+b+"'))")
ps("$u=(Get-CimInstance Win32_ComputerSystem).UserName; schtasks /create /tn kfstage /tr 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\stage.ps1' /sc once /st 23:59 /ru $u /it /f; schtasks /run /tn kfstage")
