#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# test_hook.sh — drive the app lane's harness OFFLINE (no GPU, no guest, no box): apps_hook.sh and
# hmm0_hook.sh through a fake guest ssh (APPS_GSSH), and apps_matrix.sh's guest path through a fake
# boot_capture.sh in a scratch copy of scripts/. The fake guest answers the hook's guest commands from
# scripted per-app fixtures, and appends each app's kf3 lines to the boot's fake kf3 log as the app
# "runs". Asserts what the scripts WRITE (and exit with), so their own logic is under test, not a copy
# of it. Called by test_verdicts.sh (CI step "App-matrix verdict fixtures").
#
# ★ 2026-10-03 (review of 5af7e644), the behaviours pinned:
#   - the wedge probe runs after a PASS row whose guest dmesg gained an Xid (vmm_probe's ro_write
#     faults on purpose and PASSes), not only after non-PASS rows; and not after a clean PASS row;
#   - a wedged boot (the probe fails) ends the boot there, so the rows after it are not run;
#   - each row carries rc_silent_births= (RC-UNARMED/RC-NONE birth LINES in its slice), and kf3's
#     boot sentence (which names RC-UNARMED) is not counted as one; and a managed row whose slice
#     carries that sentence still scores loud=EXPECTED_LOUD through the real hook.
# ★ 2026-10-03 (review of 9390f51c), added:
#   - the wedge probe also follows a PASS row whose kf3 slice holds an RC / UNSERVICED line with NO
#     guest Xid (the guest Xid path has never run on a box); kf3's boot sentence alone triggers nothing;
#   - hmm0_hook.sh's exit line carries the status of the steps that matter (the reload, the option,
#     the ATTR probe), not `tail`'s: a failed reload reads UNMEASURED, never `rc=0`;
#   - apps_matrix.sh: a boot whose boot_capture.sh dies early is gated on ITS OWN (absent) log, not an
#     earlier boot's left at the same tag; and a reused run name (repeated APPS_BOOT_GATE lines for one
#     boot) gives the same verdict from apps_matrix.sh's exit status, summarize.py and triage.py.
# Their stubs (sudo / modprobe / dmesg / um_probe for the guest, a boot_capture.sh, a pgrep) are files
# under fixtures/, each reading its scenario from the environment ($FAKE_G, $FAKE_DIE, $FAKE_UNARMED).
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
  *uvm_disable_hmm=1*)
    # hmm0_hook.sh's guest script, run HERE with the guest's paths mapped onto the fixture and
    # sudo / modprobe / dmesg stubbed (fixtures/hmm0/bin)
    c=${cmd//\/sys\/module\/nvidia_uvm\/parameters\/uvm_disable_hmm/$F/param}
    c=${c//\/opt\/apps\/bundle\/bin\/um_probe/$FAKE_FIX/hmm0/um_probe}
    PATH="$FAKE_FIX/hmm0/bin:$PATH" bash -c "$c"; exit $? ;;
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

lines(){ python3 "$HERE/kf3_lines.py" "$@"; }
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
  printf '%s\n' "$STATUS" "$@" > "$g/$a.kf3"
}
STATUS=$(lines rc_status 0 0)   # kf3's status line at this revision (rendered from device.rs)
XID(){ echo "[   60.100000] NVRM: Xid (PCI:0000:00:02): 31, pid=77, name=$1, $(lines xid_text 1)"; }
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

# ---- 4. the probe's kf3-side trigger: an RC or UNSERVICED line with NO guest Xid -----------------
# (review of 9390f51c) R3's host RCs reached the guest with no Xid line at all; the host's own record
# that a row faulted must trigger the probe by itself. kf3's boot sentence (which names RC_TRIGGERED
# and Xid 31) must not.
app s4 bootonly PASS "" "$boot"
app s4 hostrc PASS "" "$(lines host_twin)" "$(lines posted)"
app s4 unsvc PASS "" "$(lines unserviced)"
app s4 clean PASS ""
hook s4 "bootonly hostrc unsvc clean" 1
t probe_after_a_host_rc_without_a_guest_xid "after hostrc|after unsvc" "$(paste -sd'|' "$T/s4/g/probes")"
grep -q 'APPS_HOOK wedge probe after hostrc (guest_xid=0 kf3_rc=2)' "$T/s4/hook.out"; t probe_line_names_the_kf3_trigger 0 $?

# ---- 5. hmm0_hook.sh: the exit line is the status of the steps that matter ------------------------
FIX="$HERE/fixtures"
hmm(){  # $1 scenario, $2 `modprobe -r` rc, $3 `um_probe attrs` rc, $4 its ATTR line ('' = none)
  local g="$T/$1/g"; mkdir -p "$g" "$T/$1/res"
  echo N > "$g/param"; echo "$2" > "$g/rmmod_rc"; echo "$3" > "$g/attrs_rc"; printf '%s' "$4" > "$g/attr"
  lines xid_text 1 > "$g/xidtext"; : > "$g/dmesg"
  FAKE_G="$g" FAKE_FIX="$FIX" APPS_GSSH="$T/gssh" APPS_RESULTS="$T/$1/res" \
    bash "$HERE/hmm0_hook.sh" "t_$1" < /dev/null > "$T/$1/hook.out" 2>&1
  grep -a '^HMM0_END ' "$T/$1/res/r5hmm0.txt"
}
hf(){ sed -n "s/.* $1=\([^ ]*\).*/\1/p" <<<"$2"; }
ATTR0="ATTR managed=1 concurrentManagedAccess=1 pageableMemoryAccess=0"
e=$(hmm h1 0 0 "$ATTR0")
t hmm0_measured MEASURED "$(hf status "$e")"
t hmm0_measured_fields "Y 2 1" "$(hf hmm_disabled "$e") $(hf pageable_rc "$e") $(hf new_xid "$e")"
grep -qx "$ATTR0" "$T/h1/res/r5hmm0.txt"; t hmm0_attr_line_recorded 0 $?
# the module is busy: -r fails, the load is a no-op, the option never took — `tail` still exits 0
e=$(hmm h2 1 0 "ATTR managed=1 concurrentManagedAccess=1 pageableMemoryAccess=1")
t hmm0_failed_reload_is_unmeasured "UNMEASURED 1 N" "$(hf status "$e") $(hf reload_rc "$e") $(hf hmm_disabled "$e")"
e=$(hmm h3 0 1 "")
t hmm0_no_attr_line_is_unmeasured "UNMEASURED 1 0" "$(hf status "$e") $(hf attrs_rc "$e") $(hf attr_lines "$e")"

# ---- 6. apps_matrix.sh's guest path: own-boot logs, and ONE rule for the appended gate lines ------
# a scratch copy of scripts/apps with fixtures/boot_capture.sh as scripts/bench/boot_capture.sh, and
# fixtures/pgrep first on PATH (an idle bench)
M="$T/mx"; mkdir -p "$M/repo/scripts/bench" "$M/bin" "$M/bench"
cp -r "$HERE" "$M/repo/scripts/apps"; rm -rf "$M/repo/scripts/apps/__pycache__"
cp "$FIX/boot_capture.sh" "$M/repo/scripts/bench/boot_capture.sh"; cp "$FIX/pgrep" "$M/bin/pgrep"
mx(){ PATH="$M/bin:$PATH" FAKE_FIX="$FIX" APPS_ROOT="$M/apps" APPS_RESULTS="$M/res" BENCH_DIR="$M/bench" \
        KF_LOCK="$M/lock" KF3_BIN=/bin/true bash "$M/repo/scripts/apps/apps_matrix.sh" "$@" > "$M/out.$2" 2>&1; echo $?; }
lane_of(){ sed -n 's/^BOOT_GATE .* lane=\([A-Z]*\).*/\1/p'; }
lanes(){  # run $1's lane as apps_matrix.sh exits on it (0 = PASS), summarize.py and triage.py read it
  echo "$(mx lane "$1") $(python3 "$HERE/summarize.py" "$M/res/$1" | lane_of) $(python3 "$HERE/triage.py" "$M/res/$1" | lane_of)"
}
# a PASS log left at the tag by an earlier boot; THIS boot's boot_capture.sh dies before writing one
lines rc_status 0 0 > "$M/bench/run_ap_m1_b1_qemu.log"
t matrix_dead_boot_exits_3 3 "$(FAKE_DIE=1 mx guest m1 vectorAdd)"
grep -q '^APPS_BOOT_GATE boot=ap_m1_b1 gate=UNMEASURED ' "$M/res/m1/guest.res"; t matrix_dead_boot_never_reads_a_stale_log 0 $?
# a reused run name: FAIL, then PASS on the same boot tag — the newest attempt is the verdict, everywhere
t matrix_attempt1_fails 3 "$(FAKE_UNARMED=1 mx guest m2 vectorAdd)"
t matrix_attempt2_passes 0 "$(FAKE_UNARMED=0 mx guest m2 vectorAdd)"
t matrix_repeated_gate_lines 2 "$(grep -c '^APPS_BOOT_GATE boot=ap_m2_b1 ' "$M/res/m2/guest.res")"
t matrix_one_rule_fail_then_pass "0 PASS PASS" "$(lanes m2)"
# … and PASS, then FAIL
FAKE_UNARMED=0 mx guest m3 vectorAdd >/dev/null; FAKE_UNARMED=1 mx guest m3 vectorAdd >/dev/null
t matrix_one_rule_pass_then_fail "3 FAIL FAIL" "$(lanes m3)"
# a reused run name whose second attempt's boots all die: each app is recorded BOOT_FAIL by THIS
# attempt — an earlier attempt's row at the same boot tag does not mark it done
FAKE_UNARMED=0 mx guest m4 appA appB >/dev/null
t matrix_reused_run_dead_boots_exit_3 3 "$(FAKE_DIE=1 mx guest m4 appA appB)"
t matrix_reused_run_records_its_own_boot_fails 2 "$(tail -n +5 "$M/res/m4/guest.res" | grep -c '^APPRES .* verdict=BOOT_FAIL ')"
python3 "$HERE/summarize.py" "$M/res/m4" | grep -q '| appB | - | BOOT_FAIL |'; t matrix_reused_run_summary_shows_the_new_attempt 0 $?

echo "HOOK_FIXTURES pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
