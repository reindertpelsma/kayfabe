#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# test_verdicts.sh — offline fixture test of the app matrix's release-§I classifiers (no GPU, no
# guest, no box): loud_verdict.sh, boot_gate.sh, summarize.py's and triage.py's gate/RC columns,
# and (test_hook.sh) apps_hook.sh itself.
#
# Fixtures are the R3 per-app slices (kf3 4c48ca0c, 2026-09-28) committed in
# traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/all_logs.tgz (m20/iso/*): the four managed-memory
# apps, run alone, BEFORE kf3 posted a guest Xid. So:
#   - as recorded                                   → SILENT (no guest Xid, no named host line)
#   - + a guest `Xid 31 … kayfabe:` + UNSERVICED line → EXPECTED_LOUD
#   - + a C′ `not applied` line                     → KF3_DEFECT
#   - + an `RC-NONE` birth                           → SILENT
#   - a PASS row, an unlisted row, a hang, a row whose app saw no error → as the rules say
# ★ 2026-10-03 (review of 5af7e644): every kf3 line injected here is RENDERED FROM THE RUST SOURCE
# by kf3_lines.py (the format strings and the DELIVERY_UNBUILT constant), never hand-typed — the
# hand-typed C′ line was not the shape kf3 prints, and the line that broke the classifier (kf3's own
# boot-report sentence, which names `RC-UNARMED`) was in no fixture. Every loud/defect/none slice
# now carries that boot line, as a real isolated boot's slice does.
# ⊘ CORRECTED 2026-10-03 (review of 9390f51c): rendering did not yet mean FOLLOWING THE SPEC —
# kf3_lines.py filled pre-formatted strings, so `{:#x}` → `{:x}` in chan.rs left every case green.
# It now formats typed values by each placeholder's spec, and the `spec drift` cases below mutate a
# COPY of the tree and assert the fixture moves with it (or the renderer refuses, exit 2). The status
# line's `rc[…]` counters and the guest Xid text come from the source too (`rc_status`, `xid_text`).
# Also: `bash -n` on the harness scripts, and `run_apps.sh x list` carries the release rows.
# Prints one `TEST <name> ok|FAIL` per case and `VERDICT_FIXTURES pass=<n> fail=<m>`; exit 1 on a FAIL.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
TGZ="$REPO/traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/all_logs.tgz"
LV=${LOUD_VERDICT:-$HERE/loud_verdict.sh}; BG=${BOOT_GATE:-$HERE/boot_gate.sh}
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
pass=0; fail=0
t(){ if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "TEST $1 ok"; else fail=$((fail+1)); echo "TEST $1 FAIL want=$2 got=$3"; fi; }
cls(){ bash "$LV" "$@" | sed -n 's/^LOUD=\([^ ]*\).*/\1/p'; }
gate(){ bash "$BG" "$1" | sed -n 's/^GATE=\([^ ]*\).*/\1/p'; }

for f in run_apps.sh apps_hook.sh apps_matrix.sh build_bundle.sh loud_verdict.sh boot_gate.sh hmm0_hook.sh test_verdicts.sh test_hook.sh fixtures/boot_capture.sh fixtures/hmm0/um_probe fixtures/hmm0/bin/modprobe; do
  bash -n "$HERE/$f" 2>/dev/null; t "bash_n_$f" 0 $?
done
for f in summarize.py triage.py kf3_lines.py boot_gates.py; do
  python3 -m py_compile "$HERE/$f" 2>/dev/null; t "py_compile_$f" 0 $?
done
list=" $(bash "$HERE/run_apps.sh" x list) "
for r in vectorAddMMAP vmm_probe torch_expseg um_cpuinit um_gpufirst um_pageable um_prefetch um_advise um_malloc um_hostalloc um_d2h; do
  case "$list" in *" $r "*) t "listed_$r" yes yes ;; *) t "listed_$r" yes no ;; esac
done

# ---- the kf3 lines, as kf3 prints them at this revision -------------------------------------------
L="$T/lines"; mkdir -p "$L"
for k in boot_line unarmed none posted unserviced not_applied host_twin xid_posted rc_status xid_text; do
  python3 "$HERE/kf3_lines.py" "$k" > "$L/$k" 2>"$L/$k.err"; rc=$?
  t "kf3_line_rendered_$k" 0 "$rc"; [ $rc = 0 ] || cat "$L/$k.err"
done
t kf3_unarmed_producers 3 "$(grep -c . "$L/unarmed")"
# ⊘ the reviewed bug, pinned: the boot sentence DOES name RC-UNARMED (so a bare-word grep misfires)
grep -q 'RC-UNARMED' "$L/boot_line"; t boot_line_names_rc_unarmed 0 $?

APPS="conjugateGradientUM attach_verify UnifiedMemoryPerf UnifiedMemoryStreams"
members=""; for a in $APPS; do for k in guest.log guest_dmesg.log kf3.log; do members="$members m20/iso/$a.$k"; done; done
if ! tar -xzf "$TGZ" -C "$T" m20/iso/guest.res $members m20/host.res m20/guest.res m20/guest_isolated.res 2>/dev/null; then
  t fixtures_extract ok missing; echo "VERDICT_FIXTURES pass=$pass fail=$fail"; exit 1
fi
I="$T/m20/iso"
res(){ grep -a "app=$1 " "$I/guest.res" | head -1; }
copy(){ mkdir -p "$T/$2"; for k in guest.log guest_dmesg.log kf3.log; do cp "$I/$1.$k" "$T/$2/$1.$k"; done; }
run(){ cls "$1" "$T/$2/$1.guest.log" "$T/$2/$1.guest_dmesg.log" "$T/$2/$1.kf3.log" "${3:-$(res "$1")}"; }
comm_of(){ printf '%.15s' "$1"; }   # the guest prints the task comm, cut to 15 characters
# the guest's Xid line: `NVRM: Xid (PCI:…): 31, pid=…, name=…, ` (the guest driver's prefix) + the
# text kf3 posts, rendered from chan.rs `xid_text`
gxid(){ echo "[   52.218000] NVRM: Xid (PCI:0000:00:02): 31, pid=4242, name=$(comm_of "$1"), $(cat "$L/xid_text")"; }

for a in $APPS; do
  copy "$a" asis; t "r3_asis_$a" SILENT "$(run "$a" asis)"
  # ★ BLOCKER fixture (review of 5af7e644): the boot sentence alone must not make a row KF3_DEFECT
  copy "$a" boot; cat "$L/boot_line" >> "$T/boot/$a.kf3.log"
  t "r3_bootline_still_silent_$a" SILENT "$(run "$a" boot)"
  copy "$a" loud
  cat "$L/boot_line" >> "$T/loud/$a.kf3.log"
  gxid "$a" >> "$T/loud/$a.guest_dmesg.log"
  cat "$L/unserviced" >> "$T/loud/$a.kf3.log"
  t "r3_loud_$a" EXPECTED_LOUD "$(run "$a" loud)"
  copy "$a" defect; cp "$T/loud/$a.guest_dmesg.log" "$T/loud/$a.kf3.log" "$T/defect/"
  cat "$L/not_applied" >> "$T/defect/$a.kf3.log"
  t "r3_defect_$a" KF3_DEFECT "$(run "$a" defect)"
  copy "$a" none; cp "$T/loud/$a.guest_dmesg.log" "$T/loud/$a.kf3.log" "$T/none/"
  cat "$L/none" >> "$T/none/$a.kf3.log"
  t "r3_rcnone_$a" SILENT "$(run "$a" none)"
done

# every RC-UNARMED producer is the C′ class's sibling: the row is a kf3 defect, not paging
n=0
while IFS= read -r u; do
  n=$((n+1)); d="unarmed$n"
  copy attach_verify "$d"; cp "$T/loud/attach_verify.guest_dmesg.log" "$T/loud/attach_verify.kf3.log" "$T/$d/"
  echo "$u" >> "$T/$d/attach_verify.kf3.log"
  t "unarmed_is_defect_$n" KF3_DEFECT "$(run attach_verify "$d")"
done < "$L/unarmed"
# the boot sentence names RC_TRIGGERED too: it must not stand in for a posted one
copy attach_verify noposted; cp "$T/loud/attach_verify.guest_dmesg.log" "$T/noposted/"
grep -av 'RC_TRIGGERED posted' "$T/loud/attach_verify.kf3.log" > "$T/noposted/attach_verify.kf3.log"
t bootline_is_not_a_posted_rc SILENT "$(run attach_verify noposted)"

# §I's literal requirement: the APP must see an error — waived only for conjugateGradientUM
copy attach_verify quiet; cp "$T/loud/attach_verify.guest_dmesg.log" "$T/loud/attach_verify.kf3.log" "$T/quiet/"
printf 'attach_verify: 4 buffers x 1048576 elements, 0 mismatched\n' > "$T/quiet/attach_verify.guest.log"
t app_saw_no_error SILENT "$(run attach_verify quiet "APPRES side=guest app=attach_verify verdict=FAIL rc=0 secs=6 quiet=0 note=-")"
t cgum_waiver EXPECTED_LOUD "$(run conjugateGradientUM loud "APPRES side=guest app=conjugateGradientUM verdict=FAIL rc=0 secs=11 quiet=1 note=Test Summary: Error amount = 1.000000, result = SUCCESS rc=0")"
# a note is free text: an `rc=` inside it must not stand in for the row's own rc
t note_rc_ignored SILENT "$(run attach_verify quiet "APPRES side=guest app=attach_verify verdict=FAIL rc=0 secs=6 quiet=0 note=x rc=139")"

# a hang is never loud, even with every marker present
t hang_is_silent SILENT "$(run attach_verify loud "APPRES side=guest app=attach_verify verdict=TIMEOUT rc=124 secs=90 quiet=85 note=-")"
t guest_dead_is_silent SILENT "$(run attach_verify loud "APPRES side=guest app=attach_verify verdict=GUEST_DEAD rc=ssh255 secs=- quiet=- note=no-result-line(guest_alive=0)")"
# PASS stays PASS; an unlisted row is not this classifier's; a row that never ran is untested
t pass_unchanged PASS "$(run attach_verify asis "APPRES side=host app=attach_verify verdict=PASS rc=0 secs=4 quiet=0 note=-")"
t unlisted_row - "$(cls vectorAdd /dev/null /dev/null /dev/null "APPRES side=guest app=vectorAdd verdict=FAIL rc=1 secs=1 quiet=0 note=-")"
t notrun_untested UNTESTED "$(run um_cpuinit asis "APPRES side=guest app=um_cpuinit verdict=NOTRUN rc=127 secs=0 quiet=0 note=-")"
# the deterministic proof rows: um_probe's own FAIL line carries the 719
mkdir -p "$T/um"; cp "$T/loud/attach_verify.guest_dmesg.log" "$T/um/um_cpuinit.guest_dmesg.log"; cp "$T/loud/attach_verify.kf3.log" "$T/um/um_cpuinit.kf3.log"
printf 'ATTR managed=1 concurrentManagedAccess=1 pageableMemoryAccess=1\nCHECK cpuinit FAIL cudaDeviceSynchronize() -> 719 (unspecified launch failure)\n' > "$T/um/um_cpuinit.guest.log"
t um_cpuinit_loud EXPECTED_LOUD "$(run um_cpuinit um "APPRES side=guest app=um_cpuinit verdict=FAIL rc=2 secs=3 quiet=0 note=CHECK cpuinit FAIL")"

# ---- boot_gate.sh: every apps boot asserts rc[unarmed=0 none=0] ----------------------------------
G="$T/gate"; mkdir -p "$G"
K="$I/attach_verify.kf3.log"
# a status line of THIS revision (R3's lines predate the `none=` counter), rendered from device.rs
modern(){ python3 "$HERE/kf3_lines.py" rc_status "$1" "$2"; }
cp "$K" "$G/r3.log"
t gate_r3_predates_none_counter UNMEASURED "$(gate "$G/r3.log")"
t gate_missing_log UNMEASURED "$(gate "$G/absent.log")"
{ cat "$K" "$L/boot_line"; modern 0 0; } > "$G/pass.log"
t gate_pass_with_the_boot_sentence PASS "$(gate "$G/pass.log")"
{ cat "$G/pass.log" "$L/none"; } > "$G/none_birth.log"
t gate_rcnone_birth_fails FAIL "$(gate "$G/none_birth.log")"
{ cat "$K" "$L/boot_line"; head -1 "$L/unarmed"; modern 1 0; } > "$G/unarmed.log"
t gate_unarmed_fails FAIL "$(gate "$G/unarmed.log")"
# a birth line printed after the last status line (the status prints every 2 s): the LINE alone fails
# the gate — so the birth regex itself is under test, not only the counters (review of 9390f51c)
n=0
while IFS= read -r u; do
  n=$((n+1)); { cat "$K" "$L/boot_line"; modern 0 0; echo "$u"; } > "$G/unarmed_birth.log"
  t "gate_unarmed_birth_alone_fails_$n" FAIL "$(gate "$G/unarmed_birth.log")"
done < "$L/unarmed"
{ cat "$K" "$L/boot_line"; modern 0 1; } > "$G/none_count.log"
t gate_none_counter_fails FAIL "$(gate "$G/none_count.log")"
{ cat "$K" "$L/none"; } > "$G/birth_no_status.log"
t gate_birth_without_status_fails FAIL "$(gate "$G/birth_no_status.log")"
# the counters are cumulative for the boot: the LAST status line is the boot's count, not the first
{ cat "$K"; modern 0 0; modern 0 1; } > "$G/last_wins.log"
t gate_last_status_counts FAIL "$(gate "$G/last_wins.log")"
zstd -q -c "$G/pass.log" > "$G/pass.log.zst" 2>/dev/null
t gate_reads_zst PASS "$(gate "$G/pass.log.zst")"

# ---- spec drift: the fixtures FOLLOW the format spec, or the renderer refuses ---------------------
# (review of 9390f51c) a COPY of the four source files, mutated; the real tree is never touched
D="$T/drift"
fresh(){ rm -rf "$D"; for f in crates/kf-qemu/src/chan.rs crates/kf-qemu/src/device.rs crates/kf-mem/src/vasmgr.rs crates/kf-abi/src/faultbuffer.rs; do
  mkdir -p "$D/$(dirname "$f")"; cp "$REPO/$f" "$D/$f"; done; }
drender(){ KF3_LINES_ROOT="$D" python3 "$HERE/kf3_lines.py" "$@" 2>"$T/drift.err"; }
fresh; sed -i 's/{:#x}:{:#x} RC-UNARMED/{:x}:{:x} RC-UNARMED/; s/{client:#x}:{handle:#x} RC-UNARMED/{client:x}:{handle:x} RC-UNARMED/' "$D/crates/kf-qemu/src/chan.rs"
drender unarmed > "$T/drift.unarmed"
t drift_hex_spec_moves_the_fixture 3 "$(grep -c '^kf3: chan c1d0001e:caf00099 RC-UNARMED: ' "$T/drift.unarmed")"
# … and the fixture built from it is no longer a defect: unarmed_is_defect_* would FAIL on such a tree
copy attach_verify drift; cp "$T/loud/attach_verify.guest_dmesg.log" "$T/loud/attach_verify.kf3.log" "$T/drift/"
head -1 "$T/drift.unarmed" >> "$T/drift/attach_verify.kf3.log"
t drift_hex_spec_would_fail_unarmed_is_defect EXPECTED_LOUD "$(run attach_verify drift)"
fresh; sed -i 's/{:#x}:{:#x} RC-NONE/{:#010x}:{:#x} RC-NONE/' "$D/crates/kf-qemu/src/chan.rs"
drender none >/dev/null; t drift_unmodelled_spec_refused 2 $?
fresh; sed -i 's/kf3: mem t={:.3}s REFUSED/kf3: mem t={}s REFUSED/' "$D/crates/kf-qemu/src/device.rs"
drender not_applied >/dev/null; t drift_float_display_refused 2 $?
fresh; sed -i 's/^#\[derive(Debug, \(.*\))\]$/#[derive(\1)]/' "$D/crates/kf-mem/src/vasmgr.rs"
drender not_applied >/dev/null; t drift_vaskey_debug_refused 2 $?
fresh; sed -i 's/ rc\[armed={} unarmed={} none={} wakes={}/ rc[armed={} unarmed={} none={}/' "$D/crates/kf-qemu/src/device.rs"
drender rc_status >/dev/null; t drift_positional_count_refused 2 $?

# ---- summarize.py / triage.py: the lane's verdict -------------------------------------------------
S="$T/sum"; mkdir -p "$S/iso"
row(){ echo "APPRES side=guest app=$1 verdict=$2 rc=0 secs=1 quiet=0 note=- boot=$3 guest_xid=0 kf3_lines=1 kf3_refusals=0 rc_unarmed=0 rc_none=0 rc_silent_births=${4:-0} loud=-"; }
gl(){ echo "APPS_BOOT_GATE boot=$1 gate=$2 unarmed=0 none=0 births=0 why=x"; }
{ row vectorAdd PASS b1; row matrixMul PASS b1; gl b1 PASS; row deviceQuery PASS b2; gl b2 PASS; } > "$S/guest.res"
lane(){ python3 "$HERE/summarize.py" "$1" | sed -n 's/^BOOT_GATE .* lane=\([A-Z]*\).*/\1/p'; }
t summarize_all_gates_pass PASS "$(lane "$S")"
{ row vectorAdd PASS b1; row vmm_probe PASS b1 1; gl b1 FAIL; } > "$S/guest.res"
t summarize_a_failed_gate_fails_the_lane FAIL "$(lane "$S")"
python3 "$HERE/summarize.py" "$S" | grep -q '| vmm_probe | - | PASS/RC_SILENT |'; t summarize_row_shows_rc_silent 0 $?
{ row vectorAdd PASS b1; gl b1 PASS; row matrixMul PASS b2; } > "$S/guest.res"
t summarize_a_boot_without_a_gate_line_fails FAIL "$(lane "$S")"
{ row vectorAdd PASS b1; gl b1 PASS; } > "$S/guest.res"; { row attach_verify FAIL i1; gl i1 UNMEASURED; } > "$S/iso/guest.res"
t summarize_an_unmeasured_iso_gate_fails FAIL "$(lane "$S")"
t summarize_r3_predates_the_gate UNMEASURED "$(lane "$T/m20")"
# ⊘ (review of 9390f51c) guest.res is APPENDED to: a reused run name repeats a boot's gate line. ONE
# rule (boot_gates.py, the last line per boot) for summarize.py, triage.py and apps_matrix.sh's exit
# (`apps_matrix.sh lane`, which the guest path ends with; test_hook.sh drives the guest path itself)
rm -f "$S/iso/guest.res"
alllanes(){  # summarize.py's lane, triage.py's lane, apps_matrix.sh lane's exit status
  echo "$(lane "$S") $(python3 "$HERE/triage.py" "$S" | sed -n 's/^BOOT_GATE .* lane=\([A-Z]*\).*/\1/p') \
$(APPS_RESULTS="$T" bash "$HERE/apps_matrix.sh" lane sum >/dev/null 2>&1; echo $?)"; }
{ row vectorAdd PASS b1; gl b1 FAIL; row vectorAdd PASS b1; gl b1 PASS; } > "$S/guest.res"
t one_rule_repeated_gate_fail_then_pass "PASS PASS 0" "$(alllanes)"
{ row vectorAdd PASS b1; gl b1 PASS; row vectorAdd PASS b1; gl b1 FAIL; } > "$S/guest.res"
t one_rule_repeated_gate_pass_then_fail "FAIL FAIL 3" "$(alllanes)"
# triage's RC column: the real RC line, never kf3's boot sentence printed before it
TR="$T/tri"; mkdir -p "$TR"
{ cat "$L/boot_line"; cat "$I/attach_verify.kf3.log"; } > "$TR/attach_verify.kf3.log"
cp "$I/attach_verify.guest_dmesg.log" "$TR/"; res attach_verify > "$TR/guest.res"; gl b9 FAIL >> "$TR/guest.res"
python3 "$HERE/triage.py" "$TR" > "$TR/out" 2>&1
grep -q '|kf3rc:kf3: RC host twin 0x' "$TR/out"; t triage_rc_column_skips_the_boot_sentence 0 $?
# boot_gates.py's rule: b9's FAIL line, and the R3 row's boot that ran with no gate line (UNMEASURED)
grep -q '^BOOT_GATE boots=2 pass=0 fail=1 unmeasured=1 lane=FAIL' "$TR/out"; t triage_names_the_failed_gate 0 $?
grep -q '⊘ b9 gate=FAIL ' "$TR/out"; t triage_lists_the_failed_boot 0 $?

# ---- apps_hook.sh itself, offline (fake guest ssh) ------------------------------------------------
bash "$HERE/test_hook.sh" > "$T/hook.out" 2>&1; hrc=$?
sed 's/^/  hook: /' "$T/hook.out"
t apps_hook_offline 0 "$hrc"

echo "VERDICT_FIXTURES pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
