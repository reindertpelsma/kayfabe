#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# test_hook.sh — drive apps_hook.sh OFFLINE (no GPU, no guest, no box): a fake guest ssh (APPS_GSSH)
# answers the hook's guest commands from scripted per-app fixtures, and appends each app's kf3 lines
# to the boot's fake kf3 log as the app "runs". Asserts what the hook WRITES, so the hook's own logic
# is under test, not a copy of it. Called by test_verdicts.sh (CI step "App-matrix verdict fixtures").
#
# ★ 2026-10-03 (review of 5af7e644), the behaviours pinned:
#   - the wedge probe runs after a PASS row whose guest dmesg gained an Xid (vmm_probe's ro_write
#     faults on purpose and PASSes), not only after non-PASS rows; and not after a clean PASS row;
#   - a wedged boot (the probe fails) ends the boot there, so the rows after it are not run;
#   - each row carries rc_silent_births= (RC-UNARMED/RC-NONE birth LINES in its slice), and kf3's
#     boot sentence (which names RC-UNARMED) is not counted as one; and a managed row whose slice
#     carries that sentence still scores loud=EXPECTED_LOUD through the real hook.
# Prints `TEST <name> ok|FAIL` per case and `HOOK_FIXTURES pass=<n> fail=<m>`; exit 1 on a FAIL.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK=${APPS_HOOK:-$HERE/apps_hook.sh}
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
pass=0; fail=0
t(){ if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "TEST $1 ok"; else fail=$((fail+1)); echo "TEST $1 FAIL want=$2 got=$3"; fi; }

cat > "$T/gssh" <<'EOF'
#!/usr/bin/env bash
# the fake guest: $FAKE_G/<app>.{res,log,dmesg,kf3}; $FAKE_G/dmesg = the guest's dmesg so far;
# $FAKE_G/probe_answer = what the vectorAdd sanity probe prints (1 = "Test PASSED" once)
F=${FAKE_G:?}; cmd="$*"
case "$cmd" in
  true) exit 0 ;;
  *"run_apps.sh guest "*)
    app=${cmd##* }; echo "$app" >> "$F/ran"
    cat "$F/$app.dmesg" >> "$F/dmesg" 2>/dev/null
    cat "$F/$app.kf3" >> "$FAKE_QLOG" 2>/dev/null
    cat "$F/$app.res"; exit 0 ;;
  *"/opt/apps/out/guest/"*) a=${cmd##*/}; cat "$F/${a%.log}.log" 2>/dev/null; exit 0 ;;
  *"dmesg | wc -l"*) wc -l < "$F/dmesg"; exit 0 ;;
  *"dmesg | tail -n +"*) tail -n +"${cmd##*tail -n +}" "$F/dmesg"; exit 0 ;;
  *samples/vectorAdd*) echo "after $(tail -1 "$F/ran")" >> "$F/probes"; cat "$F/probe_answer"; exit 0 ;;
  *) cat >/dev/null 2>&1; exit 0 ;;
esac
EOF
chmod +x "$T/gssh"

lines(){ python3 "$HERE/kf3_lines.py" "$1"; }
# one scenario: $1 = name, $2 = apps, $3 = probe answer; fixtures already in $T/$1/g
hook(){
  local d="$T/$1"; mkdir -p "$d/out" "$d/bench"; : > "$d/g/dmesg"; : > "$d/g/ran"; : > "$d/g/probes"
  echo "$3" > "$d/g/probe_answer"
  FAKE_G="$d/g" FAKE_QLOG="$d/bench/run_t_$1_qemu.log" APPS_GSSH="$T/gssh" APPS_OUT="$d/out" \
    BENCH_DIR="$d/bench" APPS="$2" bash "$HOOK" "t_$1" < /dev/null > "$d/hook.out" 2>&1
}
app(){  # $1 scenario, $2 app, $3 verdict, $4 dmesg line (or ''), $5.. kf3 lines
  local g="$T/$1/g" a=$2 v=$3 dm=$4; shift 4; mkdir -p "$g"
  echo "APPRES side=guest app=$a verdict=$v rc=$([ "$v" = PASS ] && echo 0 || echo 1) secs=3 quiet=0 note=-" > "$g/$a.res"
  echo "=== $a" > "$g/$a.log"; [ "$v" = PASS ] || echo "FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure)" >> "$g/$a.log"
  if [ -n "$dm" ]; then echo "$dm" > "$g/$a.dmesg"; else : > "$g/$a.dmesg"; fi
  printf '%s\n' "kf3: family=Ampere phase=Running rc[armed=8 unarmed=0 none=0 wakes=0 seen=0 posted=0 xid=0]" "$@" > "$g/$a.kf3"
}
XID(){ echo "[   60.100000] NVRM: Xid (PCI:0000:00:02): 31, pid=77, name=$1, kayfabe: unserviced GPU page fault; 1 channel(s) of this process stopped. Common cause: CUDA managed memory or HMM pageable access (unsupported). Else an invalid GPU access by the app (as on bare metal) or a kayfabe bug: please report."; }
field(){ grep -a "app=$2 " "$T/$1/out/guest.res" | sed -n "s/.* $3=\([^ ]*\).*/\1/p"; }

# ---- 1. vmm_probe PASSes with a deliberate Xid: the probe follows it; a clean PASS gets none ----
boot=$(lines boot_line)
app s1 vmm_probe PASS "$(XID vmm_probe)" "$boot" "$(lines host_twin)" "$(lines posted)"
app s1 vectorAdd PASS ""
app s1 matrixMul PASS ""
hook s1 "vmm_probe vectorAdd matrixMul" 1
t probe_after_a_pass_row_with_an_xid "after vmm_probe" "$(cat "$T/s1/g/probes")"
t no_probe_after_a_clean_pass 1 "$(grep -c . "$T/s1/g/probes")"
t all_rows_ran 3 "$(grep -c '^APPRES ' "$T/s1/out/guest.res")"
t vmm_probe_guest_xid 1 "$(field s1 vmm_probe guest_xid)"
t bootline_is_not_a_silent_birth 0 "$(field s1 vmm_probe rc_silent_births)"
grep -q '^APPS_HOOK_DONE' "$T/s1/hook.out"; t hook_done 0 $?

# ---- 2. the deliberate fault WEDGES the boot: stop there, the later rows are not burned --------
app s2 vmm_probe PASS "$(XID vmm_probe)" "$boot"
app s2 attach_verify FAIL ""
hook s2 "vmm_probe attach_verify" 0
grep -q '^APPS_WEDGE boot=t_s2 after=vmm_probe ' "$T/s2/out/guest.res"; t wedge_recorded_after_vmm_probe 0 $?
t rows_after_the_wedge_not_run "vmm_probe" "$(tr '\n' ' ' < "$T/s2/g/ran" | sed 's/ $//')"

# ---- 3. per-row silent births, by LINE shape; a managed row is EXPECTED_LOUD through the hook --
app s3 attach_verify FAIL "$(XID attach_verify)" "$boot" "$(lines host_twin)" "$(lines unserviced)" "$(lines xid_posted)" "$(lines posted)"
app s3 stream_default PASS "" "$(lines none)"
hook s3 "attach_verify stream_default" 1
t managed_row_loud_through_the_hook EXPECTED_LOUD "$(field s3 attach_verify loud)"
t managed_row_no_silent_births 0 "$(field s3 attach_verify rc_silent_births)"
t rcnone_birth_counted_on_its_row 1 "$(field s3 stream_default rc_silent_births)"
t row_reads_none_counter 0 "$(field s3 stream_default rc_none)"

echo "HOOK_FIXTURES pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
