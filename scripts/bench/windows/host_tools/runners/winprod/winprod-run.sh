#!/bin/bash
# winprod-run.sh N — ONE interactive Windows 11 guest on kf3, PRODUCTION profile (class-B behaviour flags only,
# zero measurement flags, no KF3_NO_BATCHED_MAP). The CALLER holds /tmp/kayfabe-fastguest.lock (flock -o).
# Derived from guestlogs/gl-run-signinsnap15.sh (scripted sign-in, Edge double-click, Shorts keys). Unlike it, this
# script KEEPS THE VM RUNNING for the owner: it loops writing a heartbeat until the stop file appears, the QEMU
# dies, or it gets TERM/INT; then it stops the guest cleanly, restores the IOMMU group to DMA-FQ and exits.
#   stop:  touch /tmp/kf-stop-winprod     (or kill -TERM <runner pid, in winprod/runN/runner.pid>)
set -u
W=/var/lib/kf-windows-20261005
N=$1
REV=${KF3_REV_BIN:-6fafcc6e-win}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod.sh
RUN=$W/boundary-kayfabe-$N
O=$W/winprod/run$N
STOPF=/tmp/kf-stop-winprod
PW=${KF_GUEST_PW:?set KF_GUEST_PW (the throwaway test-guest password)}
mkdir -p $O; echo $$ > $O/runner.pid
L(){ echo "WINPROD $(date -u +%FT%T.%3NZ) $*" | tee -a $O/winprod.log; }
for v in $(compgen -e | grep '^KF3_'); do unset "$v"; done
unset WIN_TRACE WIN_GSP_OBSERVER WIN_OBS_ONLY WIN_DROP WIN_FLAGS PROD_EXTRA_FLAGS
export KF3_REV=$REV
XID0=$(dmesg | grep -c 'NVRM: Xid')
dmesg | grep 'NVRM: Xid' > $O/xid-before.txt
L "START run=$N rev=6fafcc6e bin=kf3-bins/$REV pid=$$ xid_before=$XID0 pre-qemu=$(pgrep -c qemu-system) kf3-env=[$(env | grep -c '^KF3_')]"
if [ "$(pgrep -c qemu-system)" != 0 ]; then L "REFUSED: another QEMU is running"; exit 4; fi
[ -e "$RUN" ] && { L "REFUSED: $RUN exists"; exit 5; }
Q(){ timeout 15 python3 $W/boundary-tools/qmp.py $RUN/qmp.sock "$@"; }
G(){ timeout ${GT:-40} python3 $W/boundary-tools/qmp.py $RUN/qga.sock "$@"; }
key(){ Q cmd send-key "{\"keys\":[{\"type\":\"qcode\",\"data\":\"$1\"}]}" >/dev/null 2>&1; sleep 0.25; }
alive(){ [ -n "${QPID:-}" ] && kill -0 $QPID 2>/dev/null; }
shot(){ Q cmd screendump "{\"filename\":\"$O/$1.ppm\",\"device\":\"kf0\"}" >/dev/null 2>&1 && convert $O/$1.ppm $O/$1.png 2>/dev/null; }
snap(){  # evidence at one moment: counters, xid delta, assert/refusal greps
  local T=$1 Lg=$RUN/qemu.log
  { echo "== snap $T $(date -u +%FT%T)  alive=$(alive && echo 1 || echo 0)"
    grep -a "phase=Running" $Lg | tail -1 | grep -a -o -E '(inval|walks|cleared|superseded|named_missed|unreconciled|absent_cleared|mapped|unmapped|held|refused|births|pt_births|acts|batch[a-z_]*|batched[a-z_]*|micro[a-z_]*)=[^ ]+' | tr '\n' ' '; echo
    echo "xid_now=$(dmesg | grep -c 'NVRM: Xid') (before=$XID0)"
    dmesg | grep 'NVRM: Xid' | diff $O/xid-before.txt - | grep '^>' | cut -c1-220
    echo "gpu_vaspace_asserts=$(grep -a -c 'gpu_vaspace' $Lg) walk_REFUSED=$(grep -a -c 'walk REFUSED' $Lg) DEAD=$(grep -a -c 'DEAD:' $Lg) too_many_pieces=$(grep -a -c 'too_many_pieces' $Lg) panics=$(grep -a -c -i 'panicked' $Lg) fps_lines=$(grep -a -c 'display fps' $Lg)"
    grep -a "display fps" $Lg | tail -1 | cut -c1-300
  } >> $O/evidence.log
}
guest_ev(){  # read-only guest-side evidence through QGA
  local T=$1
  { echo "== guest $T $(date -u +%FT%T)"
    GT=60 G qga-exec powershell.exe -NoProfile -Command '
      "display devices:"; Get-PnpDevice -Class Display | Format-Table Status,FriendlyName,ProblemCode -Auto | Out-String
      "nvidia-smi:"; & "$env:SystemRoot\System32\nvidia-smi.exe" --query-gpu=name,driver_version,display_active,utilization.gpu --format=csv 2>&1 | Out-String
      "TDR 4101 count: " + @(Get-WinEvent -FilterHashtable @{LogName="System";Id=4101} -ErrorAction SilentlyContinue).Count
      "nvlddmkm events (last 10):"; Get-WinEvent -FilterHashtable @{LogName="System";ProviderName="nvlddmkm"} -MaxEvents 10 -ErrorAction SilentlyContinue | Format-Table TimeCreated,Id -Auto | Out-String
      "bugchecks 1001/41: " + @(Get-WinEvent -FilterHashtable @{LogName="System";Id=1001,41} -ErrorAction SilentlyContinue).Count
      "uptime: " + ((Get-Date) - (gcim Win32_OperatingSystem).LastBootUpTime).ToString()
      "edge procs: " + @(Get-Process msedge -ErrorAction SilentlyContinue).Count' 2>&1 | tr -d '\r'
  } >> $O/guest-evidence.log
}
cleanup(){
  trap - TERM INT
  L "CLEANUP begin (alive=$(alive && echo 1 || echo 0))"
  snap final
  shot final-screen
  if alive; then bash $B stop $N 2>&1 | tail -3 | tee -a $O/winprod.log; fi
  for j in $(seq 1 60); do alive || break; sleep 2; done
  if alive; then Q cmd quit >/dev/null 2>&1; sleep 5; L "clean stop failed: QMP quit"; fi
  sleep 3
  L "qemu left: $(pgrep -c qemu-system)"
  bash $W/pti-iommu_nogdm.sh DMA-FQ 2>&1 | tee -a $O/winprod.log
  GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
  snap after-stop
  L "END type=$(cat /sys/kernel/iommu_groups/$GRP/type) driver=$(basename $(readlink /sys/bus/pci/devices/0000:01:00.0/driver)) xid_lines=$(dmesg | grep -c 'NVRM: Xid')"
  rm -f $STOPF
  exit 0
}
trap cleanup TERM INT

# ---- host GPU state ----
L "gpu: $(nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv,noheader) persistenced=$(systemctl is-enabled nvidia-persistenced 2>&1)/$(systemctl is-active nvidia-persistenced 2>&1)"
bash $W/pti-iommu_nogdm.sh identity 2>&1 | tee -a $O/winprod.log
GRP=$(basename "$(readlink /sys/bus/pci/devices/0000:01:00.0/iommu_group)")
T=$(cat /sys/kernel/iommu_groups/$GRP/type)
if [ "$T" != identity ]; then L "REFUSED: group $GRP is $T"; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 3; fi
rm -f $STOPF
# ---- launch ----
WIN_REUSE=0 bash $B run $N > $W/wr-launch-$N.log 2>&1 || { L "LAUNCH FAILED"; tail -20 $W/wr-launch-$N.log | tee -a $O/winprod.log; bash $W/pti-iommu_nogdm.sh DMA-FQ; exit 2; }
QPID=$(sed -n 's/^QPID=//p' $W/windows-broker-run$N.state)
L "launched qpid=$QPID"
python3 -I -c "import json;d=json.load(open('$RUN/command.json'));print('ENV_PASSED',' '.join(sorted(d['flags'])))" | tee -a $O/winprod.log
cp $RUN/command.json $O/command.json
# ---- sign in ----
TA=0; for j in $(seq 1 90); do alive || break; G qga-ping >/dev/null 2>&1 && { TA=$(date +%s); break; }; sleep 2; done
L "qga answered: ta=$TA alive=$(alive && echo 1 || echo 0)"
if [ "$TA" != 0 ] && alive; then
  G qga-exec net.exe user vast $PW >/dev/null 2>&1; L "password set rc=$?"
  while alive && [ $(( $(date +%s) - TA )) -lt ${SIGNIN_DELAY:-30} ]; do sleep 1; done
  if alive; then
    shot pre-signin
    L "SIGNIN keys at +$(( $(date +%s) - TA ))s"
    key spc; sleep 3
    for c in k f s i g n 7; do key $c; done; key ret
    L "SIGNIN sent"
    sleep 25; shot after-signin; snap after-signin
    ev(){ Q cmd input-send-event "{\"events\":$1}" >/dev/null 2>&1; sleep 0.15; }
    ev '[{"type":"abs","data":{"axis":"x","value":802}},{"type":"abs","data":{"axis":"y","value":4854}}]'
    for k in 1 2; do ev '[{"type":"btn","data":{"down":true,"button":"left"}}]'; ev '[{"type":"btn","data":{"down":false,"button":"left"}}]'; done
    L "EDGE double-click sent (abs 802,4854)"
    sleep 2; key ret; L "EDGE Enter sent"
    sleep 25; shot after-edge
    if alive; then
      Q cmd send-key '{"keys":[{"type":"qcode","data":"ctrl"},{"type":"qcode","data":"l"}]}' >/dev/null 2>&1; sleep 1
      for c in y o u t u b e dot c o m slash s h o r t s; do key $c; done; key ret
      L "SHORTS url typed"
      for n in $(seq 1 ${SHORTS_N:-6}); do sleep 9; key down; alive || break; done
      L "SHORTS scrolled"
    fi
    sleep 15; shot after-shorts; snap after-shorts; guest_ev after-shorts
    L "READY alive=$(alive && echo 1 || echo 0)"
  fi
fi
# ---- hold for the owner ----
while alive && [ ! -e $STOPF ]; do
  echo "$(date -u +%FT%T) qemu=$QPID alive=1 xid=$(dmesg | grep -c 'NVRM: Xid')" > $O/heartbeat
  I=$((${I:-0}+1)); [ $((I % 6)) = 0 ] && snap periodic
  sleep 10
done
L "HOLD ended: alive=$(alive && echo 1 || echo 0) stopfile=$([ -e $STOPF ] && echo 1 || echo 0)"
cleanup
