from pathlib import Path
w=Path('/var/lib/kf-windows-20261005')
s=(w/'codex-boundary647.sh').read_text().replace('647','648')
s=s.replace('WIN_FLAGS=KF3_RPC_TRACE=1 WIN_TRACE=1 WIN_GSP_OBSERVER=1','WIN_FLAGS= WIN_TRACE=0 WIN_GSP_OBSERVER=0')
s=s.replace('/tmp/kf-stage-generic.py','/tmp/kf-stage-normal.py')
s=s.replace('sleep 35','sleep 50')
s=s.replace("G qga-exec powershell.exe -NoProfile -Command 'Get-Content C:\\kf\\io-stage.txt' > \"$W/codex-boundary648-io.txt\"", "G qga-exec powershell.exe -NoProfile -Command 'Get-CimInstance Win32_PnPEntity | Where-Object Name -Like \"*NVIDIA*\" | Select-Object Name,ConfigManagerErrorCode' > \"$W/codex-boundary648-adapter.txt\"")
s=s[:s.index('while [ ! -e /tmp/kf-stop-boundary-648')]+'''grep -q 'KFGL RESULT OK' "$W/codex-boundary648-stage-out.txt"
grep -q '^EXIT 0' "$W/codex-boundary648-stage-out.txt"
echo NORMAL_PROFILE_GEARS_PASS
'''
(w/'codex-boundary648.sh').write_text(s)
s=Path('/tmp/kf-stage-generic.py').read_text().replace('/tmp/kf_glgears_stage.exe','/tmp/kf_glgears_normal.exe')
start=s.index('s=r"""');end=s.index('"""',start+6)
s=s[:start]+'''s=r"""& C:\kf\kf_glgears_stage.exe --seconds 45 > C:\kf\stage-out.txt 2> C:\kf\stage-err.txt; Add-Content C:\kf\stage-out.txt ('EXIT '+$LASTEXITCODE)"""'''+s[end+3:]
Path('/tmp/kf-stage-normal.py').write_text(s)
