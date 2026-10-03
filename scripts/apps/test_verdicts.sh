#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# test_verdicts.sh — offline fixture test of loud_verdict.sh (no GPU, no guest, no box).
#
# Fixtures are the R3 per-app slices (kf3 4c48ca0c, 2026-09-28) committed in
# traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/all_logs.tgz (m20/iso/*): the four managed-memory
# apps, run alone, BEFORE kf3 posted a guest Xid. So:
#   - as recorded                                   → SILENT (no guest Xid, no named host line)
#   - + a guest `Xid 31 … kayfabe:` + UNSERVICED line → EXPECTED_LOUD
#   - + a C′ `not applied` line                     → KF3_DEFECT
#   - + an `RC-NONE` birth                           → SILENT
#   - a PASS row, an unlisted row, a hang, a row whose app saw no error → as the rules say
# Also: `bash -n` on the harness scripts, and `run_apps.sh x list` carries the release rows.
# Prints one `TEST <name> ok|FAIL` per case and `VERDICT_FIXTURES pass=<n> fail=<m>`; exit 1 on a FAIL.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
TGZ="$REPO/traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/all_logs.tgz"
LV="$HERE/loud_verdict.sh"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
pass=0; fail=0
t(){ if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "TEST $1 ok"; else fail=$((fail+1)); echo "TEST $1 FAIL want=$2 got=$3"; fi; }
cls(){ bash "$LV" "$@" | sed -n 's/^LOUD=\([^ ]*\).*/\1/p'; }

for f in run_apps.sh apps_hook.sh build_bundle.sh loud_verdict.sh test_verdicts.sh; do
  bash -n "$HERE/$f" 2>/dev/null; t "bash_n_$f" 0 $?
done
list=" $(bash "$HERE/run_apps.sh" x list) "
for r in vectorAddMMAP vmm_probe torch_expseg um_cpuinit um_gpufirst um_pageable um_prefetch um_advise um_malloc um_hostalloc um_d2h; do
  case "$list" in *" $r "*) t "listed_$r" yes yes ;; *) t "listed_$r" yes no ;; esac
done

APPS="conjugateGradientUM attach_verify UnifiedMemoryPerf UnifiedMemoryStreams"
members=""; for a in $APPS; do for k in guest.log guest_dmesg.log kf3.log; do members="$members m20/iso/$a.$k"; done; done
if ! tar -xzf "$TGZ" -C "$T" m20/iso/guest.res $members 2>/dev/null; then
  t fixtures_extract ok missing; echo "VERDICT_FIXTURES pass=$pass fail=$fail"; exit 1
fi
I="$T/m20/iso"
res(){ grep -a "app=$1 " "$I/guest.res" | head -1; }
copy(){ mkdir -p "$T/$2"; for k in guest.log guest_dmesg.log kf3.log; do cp "$I/$1.$k" "$T/$2/$1.$k"; done; }
run(){ cls "$1" "$T/$2/$1.guest.log" "$T/$2/$1.guest_dmesg.log" "$T/$2/$1.kf3.log" "${3:-$(res "$1")}"; }
comm_of(){ printf '%.15s' "$1"; }   # the guest prints the task comm, cut to 15 characters

for a in $APPS; do
  copy "$a" asis; t "r3_asis_$a" SILENT "$(run "$a" asis)"
  copy "$a" loud
  echo "[   52.218000] NVRM: Xid (PCI:0000:00:02): 31, pid=4242, name=$(comm_of "$a"), kayfabe: GPU MMU fault; 8 channel(s) of this process stopped. kayfabe services no GPU page faults: CUDA managed memory or pageable (HMM) access to a non-resident page is unsupported. Otherwise this is a kayfabe bug - please report." >> "$T/loud/$a.guest_dmesg.log"
  echo "kf3: UNSERVICED-GPU-FAULT guest client 0xc1d0001e chids [0xb 0x8] host Xid 31 — kayfabe services no GPU page faults; guest gets RC_TRIGGERED + Xid 31" >> "$T/loud/$a.kf3.log"
  t "r3_loud_$a" EXPECTED_LOUD "$(run "$a" loud)"
  copy "$a" defect; cp "$T/loud/$a.guest_dmesg.log" "$T/loud/$a.kf3.log" "$T/defect/"
  echo "kf3: REFUSED VasKey(0x1) root 0x201000: 1 run(s) not applied: map 0x7c70d8400000+0x10000: Other(31)" >> "$T/defect/$a.kf3.log"
  t "r3_defect_$a" KF3_DEFECT "$(run "$a" defect)"
  copy "$a" none; cp "$T/loud/$a.guest_dmesg.log" "$T/loud/$a.kf3.log" "$T/none/"
  echo "kf3: chan 0xc1d0001e:0xcaf00099 RC-NONE: the guest declared no error notifier — this twin's faults are silent" >> "$T/none/$a.kf3.log"
  t "r3_rcnone_$a" SILENT "$(run "$a" none)"
done

# the rc[unarmed] birth line is the C′ class's sibling: the row is a kf3 defect, not paging
copy attach_verify unarmed; cp "$T/loud/attach_verify.guest_dmesg.log" "$T/loud/attach_verify.kf3.log" "$T/unarmed/"
echo "kf3: chan 0xc1d0001e:0xcaf00099 RC-UNARMED: notifier read view: refused" >> "$T/unarmed/attach_verify.kf3.log"
t unarmed_is_defect KF3_DEFECT "$(run attach_verify unarmed)"

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

echo "VERDICT_FIXTURES pass=$pass fail=$fail"
[ "$fail" -eq 0 ]
