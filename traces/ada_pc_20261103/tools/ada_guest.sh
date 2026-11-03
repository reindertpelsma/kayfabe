#!/bin/bash
# ada_guest.sh <arm> — the guest steps of one Windows boot (ada_win.py must be running for <arm>).
# Every step's output goes to <A>/steps/<n>_<name>.txt; one summary line per step in <A>/guest_summary.log.
set -uo pipefail
D=/data/claude-ada-20261103
ARM=$1; A=$D/win-$ARM; S=$A/steps; mkdir -p $S
IDLE_S=${IDLE_S:-600}
SUM=$A/guest_summary.log
n=0
say(){ echo "$(date -u +%FT%TZ) $*" | tee -a $SUM; }
G(){ # name timeout ps
  n=$((n+1)); local f=$S/$(printf %02d $n)_$1.txt
  python3 $D/agent.py $A/agent run "$3" "$2" > "$f" 2>&1; local rc=$?
  say "STEP $1 rc=$rc $(grep -E 'GATE_DISPLAY|GATE_DESKTOP|GATE_PROBE exe|GATE_TDR|KFDX RESULT|CUP8 VERDICT|bad=|KFGL gears|KFGL RESULT|GEARS_END|^GPU 0|TIMEOUT waiting|ALIVE' "$f" | tr '\n' ' ' | cut -c1-400)"
  return $rc
}
alive(){ kill -0 "$(python3 -c "import json;print(json.load(open('$A/owned.json'))['qemu'])")" 2>/dev/null; }
shot(){ python3 /var/lib/kf-windows-20261005/boundary-tools/qmp.py $A/qmp.sock cmd screendump "{\"filename\": \"$A/$1.ppm\", \"device\": \"kf0\"}" >/dev/null 2>&1 \
  && python3 -c "from PIL import Image;im=Image.open('$A/$1.ppm');im.thumbnail((960,540));im.save('$A/$1.png')" && rm -f $A/$1.ppm; say "SHOT $1 $(ls $A/$1.png 2>/dev/null)"; }

say "GUEST_START arm=$ARM idle_s=$IDLE_S"
t0=$(date +%s)
until python3 $D/agent.py $A/agent run 'Write-Output ALIVE' 15 2>/dev/null | grep -q ALIVE; do
  alive || { say "GUEST_ABORT qemu gone before the agent answered"; exit 3; }
  [ $(( $(date +%s) - t0 )) -gt 1200 ] && { say "GUEST_ABORT agent silent 1200 s"; shot no_agent; exit 4; }
  sleep 10
done
say "AGENT_UP after $(( $(date +%s) - t0 )) s"
shot desktop_first
G fetch 180 '$ProgressPreference="SilentlyContinue"; New-Item -ItemType Directory -Force C:\kf\probes | Out-Null; foreach($f in "cup2.exe","cup3.exe","cup8.exe","kf_dxprobe.exe","kf_glgears.exe"){ Invoke-WebRequest -UseBasicParsing "http://10.0.2.2:8766/$f" -OutFile "C:\kf\probes\$f" }; foreach($f in "gate_state.ps1","gate_run.ps1","gears_it.ps1"){ Invoke-WebRequest -UseBasicParsing "http://10.0.2.2:8766/$f" -OutFile "C:\kf\$f" }; Get-FileHash C:\kf\probes\*.exe | ForEach-Object { "$($_.Hash.ToLower()) $([IO.Path]::GetFileName($_.Path))" }'
G desktop 120 '& C:\kf\gate_state.ps1 -Mode desktop'
BOOT=$(grep -o 'GATE_BOOT utc=[^ ]*' $S/*_desktop.txt | head -1 | sed 's/.*utc=//')
say "GUEST_BOOT_UTC $BOOT"
G smi_L 120 '& "C:\Windows\System32\nvidia-smi.exe" -L'
G smi_q 180 '& "C:\Windows\System32\nvidia-smi.exe" -q'
probes(){ # suffix
  G cup8_$1 240 '& C:\kf\gate_run.ps1 -Exe C:\kf\probes\cup8.exe -TimeoutS 180'
  G d3d11_$1 240 '& C:\kf\gate_run.ps1 -Exe C:\kf\probes\kf_dxprobe.exe -Arg d3d11 -TimeoutS 180'
  G cup3_$1 200 '& C:\kf\gate_run.ps1 -Exe C:\kf\probes\cup3.exe -TimeoutS 120'
  G d3d12_$1 240 '& C:\kf\gate_run.ps1 -Exe C:\kf\probes\kf_dxprobe.exe -Arg d3d12 -TimeoutS 180'
  G gears_$1 200 "& C:\\kf\\gears_it.ps1 -Seconds 90 -Tag $1"
  G display_$1 60 '& C:\kf\gate_state.ps1 -Mode display'
}
probes pre
shot after_probes_pre
say "IDLE_START ${IDLE_S}s"
i0=$(date +%s)
while [ $(( $(date +%s) - i0 )) -lt $IDLE_S ]; do
  alive || { say "GUEST_ABORT qemu gone during idle"; exit 5; }
  sleep 60
  G idle_display 60 '& C:\kf\gate_state.ps1 -Mode display' || say "IDLE agent miss"
done
say "IDLE_END"
shot after_idle
probes post
shot after_probes_post
G tdr 180 "& C:\\kf\\gate_state.ps1 -Mode tdr -Since $BOOT"
G wer 180 '& C:\kf\gate_state.ps1 -Mode wer'
G smi_q_post 180 '& "C:\Windows\System32\nvidia-smi.exe" -q'
G desktop_post 120 '& C:\kf\gate_state.ps1 -Mode desktop'
say "GUEST_DONE secs=$(( $(date +%s) - t0 ))"
