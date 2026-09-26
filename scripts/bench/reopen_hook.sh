#!/usr/bin/env bash
# ★★★★★ w828 REOPEN HOOK — M sequential device lifetimes inside ONE fat-guest boot.
#
# The defect it exists for: `[measured 8dd2bbdf, vh, run_cl_U_cup2_3]` nvidia-smi opened and
# closed the device (RM tore the adapter down), then the next process's open re-ran
# `RmInitAdapter` and died `_kgspBootGspRm: unexpected WPR2 already up` — 1 boot in 12. A stock
# OS runs nvidia-smi then the app with no persistence mode, so EVERY close must leave the GSP
# re-bootable. One boot of the ladder exercises two closes; this exercises 2*M.
#
# Per iteration i (each a separate process, so each is a whole RmInitAdapter/RmShutdownAdapter):
#   nvidia-smi -L            → SMI_RC
#   cup3 (cuInit → cuCtxCreate → launch → 43)  → CUP3_VAL
# graded `REOPEN_ROW i=… smi_rc=0 cup3=43`. Then the guest driver's own evidence, COUNTED:
# every teardown-path failure message the w828 root cause produced (a zero is printed as a
# number, never omitted — "no line" and "zero lines" must not look alike).
#
# usage: POST_CAPTURE_HOOK=scripts/bench/reopen_hook.sh boot_capture.sh <tag>   (M via REOPEN_M)
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
M=${REOPEN_M:-20}
# REOPEN_CUDA=0: the GSP-only lane — each iteration is TWO nvidia-smi processes (two whole
# RmInitAdapter/RmShutdownAdapter cycles) and no CUDA, so the verdict isolates GSP teardown →
# re-boot from the memory and channel planes a CUDA context drags in.
CUDA=${REOPEN_CUDA:-1}
PER=${REOPEN_PER_TIMEOUT:-60}
die() { echo "★ reopen hook FAILED: $*"; echo "REOPEN_VERDICT=FAIL"; exit 2; }

$G 'cat > /tmp/cup3.c' < "$SRC_DIR/cup3.c" || die "could not push cup3.c"
$G 'rm -f /tmp/cup3; gcc -O0 -o /tmp/cup3 /tmp/cup3.c -lcuda 2>&1; echo GCC_CUP3_RC=$?'
$G 'test -x /tmp/cup3' || die "cup3 did not build in the guest"

# The whole loop runs IN the guest (one ssh), with a START marker and a DONE terminator so a
# killed loop and a finished one cannot look alike.
$G "cat > /tmp/reopen_loop.sh" <<GUESTEOF
#!/bin/sh
echo "REOPEN_START \$(date -Is) m=$M"
i=1
while [ \$i -le $M ]; do
  t0=\$(date +%s%N)
  timeout -k 5 $PER nvidia-smi -L >/tmp/smi.out 2>&1; src=\$?
  t1=\$(date +%s%N)
  if [ $CUDA = 1 ]; then ( cd /tmp && timeout -k 5 $PER ./cup3 ) >/tmp/cup3.out 2>&1; crc=\$?
  else timeout -k 5 $PER nvidia-smi -q -d MEMORY >/tmp/cup3.out 2>&1; crc=\$?; fi
  t2=\$(date +%s%N)
  if [ $CUDA = 1 ]; then val=\$(sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p' /tmp/cup3.out | tail -1); else val=smi2; fi
  fail=\$(grep -m1 '^FAIL' /tmp/cup3.out)
  echo "REOPEN_ROW i=\$i smi_rc=\$src cup3_rc=\$crc cup3=\${val:-none} smi_ms=\$(( (t1-t0)/1000000 )) cup3_ms=\$(( (t2-t1)/1000000 )) \${fail:+first_fail=[\$fail]}"
  i=\$((i+1))
done
echo "REOPEN_DONE \$(date -Is)"
GUESTEOF
# ⊘ DETACHED in the guest, polled from here. `[measured ed492871, ro_R boot 1]` running the loop
# inside the ssh session lost rows 11..20: the session ended silently (host CPU saturated by a
# concurrent cargo; ServerAlive 60 s) and SIGHUP took the guest loop with it — a HARNESS death
# that read as a device FAIL. The loop now survives its launcher; only the poll depends on ssh.
$G 'rm -f /tmp/reopen.out; setsid sh /tmp/reopen_loop.sh </dev/null >/tmp/reopen.out 2>&1 &'
# ⊘ A STALL is its own state: `[measured ed492871, ro_R2 boot 1]` a cup3 stuck IN THE KERNEL
# (state R, immune to SIGKILL) never lets the loop advance. No new row for 3*PER+60 s ⇒ STALLED.
LIMIT=$(( M * 2 * PER + 120 )); waited=0; last=-1; still=0
while [ "$waited" -lt "$LIMIT" ]; do
  $G 'grep -q "^REOPEN_DONE" /tmp/reopen.out' 2>/dev/null && break
  n=$($G 'grep -c "^REOPEN_ROW" /tmp/reopen.out' 2>/dev/null || echo "$last")
  if [ "$n" = "$last" ]; then still=$((still + 10)); else still=0; last=$n; fi
  if [ "$still" -ge $(( 3 * PER + 60 )) ]; then
    echo "REOPEN_STALLED after_row=$n — no progress for ${still}s"
    $G 'ps -eo pid,stat,etime,wchan:25,args | grep -E "cup3|nvidia-smi" | grep -v grep' 2>/dev/null | sed 's/^/REOPEN_STUCK /'
    break
  fi
  sleep 10; waited=$((waited + 10))
done
OUTF=/tmp/reopen_rows.$$
$G 'cat /tmp/reopen.out' > "$OUTF" 2>/dev/null || true
cat "$OUTF"
rows=$(grep -c '^REOPEN_ROW' "$OUTF" || true)
# In the GSP-only lane the second process is nvidia-smi, graded by its rc (never a faked 43).
want=43; [ "$CUDA" = 1 ] || want=smi2
good=$(grep -c "^REOPEN_ROW .* smi_rc=0 cup3_rc=0 cup3=$want " "$OUTF" || true)
done_=$(grep -c '^REOPEN_DONE' "$OUTF" || true)
rm -f "$OUTF"

echo "=== the guest driver's own teardown evidence (counts; 0 is printed) ==="
D=$($G 'sudo -n dmesg' 2>/dev/null)
c() { printf '%s' "$D" | grep -c -- "$1" || true; }
echo "REOPEN_DMESG_WPR2_UP=$(c 'WPR2 already up')"
echo "REOPEN_DMESG_INIT_FAILED=$(c 'RmInitAdapter failed')"
echo "REOPEN_DMESG_BCR_SPIN=$(c 'Failed to switch core to Falcon mode')"
echo "REOPEN_DMESG_DMA_POLL=$(c 's_dmaPoll_GA102')"
echo "REOPEN_DMESG_HALT_TIMEOUT=$(c 'Timeout waiting for Falcon to halt')"
echo "REOPEN_DMESG_FWSEC_SB_FAIL=$(c 'FWSEC-SB failed')"
echo "REOPEN_DMESG_BOOTER_UNLOAD_FAIL=$(c 'failed to execute Booter Unload')"
echo "REOPEN_DMESG_SUSPEND_TIMEOUT=$(c 'kgspWaitForProcessorSuspend')"
echo "REOPEN_ROWS=$rows REOPEN_GOOD=$good REOPEN_M=$M REOPEN_TERMINATED=$done_"
wpr2=$(c 'WPR2 already up'); initf=$(c 'RmInitAdapter failed')
if [ "$rows" -eq "$M" ] && [ "$good" -eq "$M" ] && [ "$done_" -eq 1 ] && [ "$wpr2" -eq 0 ] && [ "$initf" -eq 0 ]; then
  echo "REOPEN_VERDICT=PASS"
else
  echo "REOPEN_VERDICT=FAIL"
fi
