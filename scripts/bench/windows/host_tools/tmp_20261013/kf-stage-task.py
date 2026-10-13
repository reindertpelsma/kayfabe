import base64,json,subprocess,sys
w='/var/lib/kf-windows-20261005'; n=sys.argv[1]; helper=w+'/boundary-tools/qmp.py'; sock=w+'/boundary-kayfabe-'+n+'/qga.sock'
def ps(s):
 r=subprocess.run(['python3',helper,sock,'qga-exec','powershell.exe','-NoProfile','-Command',s],capture_output=True,text=True,timeout=60)
 print(r.stdout, r.stderr);return r.returncode
s="""$p=Start-Process C:\kf\kf_glgears_stage.exe -ArgumentList '--seconds','20' -PassThru -RedirectStandardOutput C:\kf\stage-out.txt -RedirectStandardError C:\kf\stage-err.txt; $p.WaitForExit(60000)|Out-Null; $p.Refresh(); Add-Content C:\kf\stage-out.txt ('EXIT '+$p.ExitCode)"""
b=base64.b64encode(s.encode('utf-8')).decode()
ps("[IO.File]::WriteAllBytes('C:\\kf\\stage.ps1',[Convert]::FromBase64String('"+b+"'))")
ps("$u=(Get-CimInstance Win32_ComputerSystem).UserName; schtasks /create /tn kfstage /tr 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\stage.ps1' /sc once /st 23:59 /ru $u /it /f; schtasks /run /tn kfstage")
