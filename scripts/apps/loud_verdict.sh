#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# loud_verdict.sh <app> <guest.log> <guest_dmesg.log> <kf3.log> <APPRES line>
#
# Release item §I (docs/OWNER_RULINGS.md, 2026-10-03): CUDA managed memory is NOT supported in a kf3
# guest, and an unsupported access must FAIL LOUDLY. This classifies one app row of the matrix:
#
#   LOUD=-             not a managed-memory row (the APPRES verdict is the whole verdict)
#   LOUD=PASS          a managed-memory row that passed (e.g. on the host lane)
#   LOUD=EXPECTED_LOUD failed the sanctioned way: every condition below holds
#   LOUD=KF3_DEFECT    the kf3 slice shows a publication defect (a fault that is OURS, not paging)
#   LOUD=SILENT        anything else, a hang included — a RELEASE BLOCKER
#   LOUD=UNTESTED      the row never ran (NOTRUN / BOOT_FAIL)
#
# EXPECTED_LOUD iff all of (docs/design/V3_APP_MATRIX.md §R5):
#   1. the guest dmesg has an Xid 31 that names kayfabe (`NVRM: Xid (PCI:…): 31, … kayfabe:`);
#   2. the kf3 slice has an `UNSERVICED-GPU-FAULT` line and at least one `RC_TRIGGERED posted`;
#   3. the kf3 slice has no `not applied`, `REFUSED VasKey` or `RC-UNARMED` line (else KF3_DEFECT —
#      the C′ signature of a refused host map, V3_APP_MATRIX.md §R2.3), and no `RC-NONE` birth (a
#      twin with no notifier: its faults are silent, so the row is SILENT);
#   4. the app saw an error: a CUDA 700/719 or CUBLAS_STATUS_EXECUTION_FAILED in its log, or a
#      nonzero exit that is not a timeout — waived ONLY for conjugateGradientUM, which discards every
#      cuBLAS status and prints SUCCESS on bare metal after any fatal fault too (main.cpp:192-199,
#      :270-272 at cuda-samples v12.5).
# Prints exactly one line: `LOUD=<class> why=<reason-without-spaces>`. Never exits nonzero on a verdict.
#
# ⊘ CORRECTED 2026-10-03 (review of 5af7e644): every kf3 condition used to be a BARE WORD grep
# (`RC-UNARMED`, `RC-NONE`, `not applied`, `UNSERVICED-GPU-FAULT`, `RC_TRIGGERED posted`). kf3 now
# prints its boot-report sentence `kf_abi::faultbuffer::DELIVERY_UNBUILT` once per QEMU process, at
# the first fault-buffer registration — the first CUDA process's `cuInit` — and that sentence names
# `RC-UNARMED`, `RC_TRIGGERED` and `Xid 31`. In every isolated boot it lands in the managed app's own
# slice, so the bare grep scored EVERY such row KF3_DEFECT: a false release blocker. Each condition
# now matches the SHAPE of the line kf3 prints for it (the format strings in crates/kf-qemu and
# crates/kf-mem; `test_verdicts.sh` renders them from the source with `kf3_lines.py`).
# ⊘ CORRECTED 2026-10-03 (review of 9390f51c): this said "so the two cannot drift", which the first
# renderer did not deliver — it filled pre-formatted values and ignored each placeholder's SPEC, so
# `{:#x}` → `{:x}` in chan.rs kept every fixture green while kf3 would print `chan c1d0001e:…`, which
# the shapes below never match. It now formats typed values by the spec (`{:#x}`, `{:?}`, `{:.3}`, …)
# or exits 2 on a spec it does not model, and `test_verdicts.sh`'s `drift_*` cases mutate a copy of the
# tree to prove the fixture moves. What still CAN drift: the value the code passes into a placeholder.
#   RC-UNARMED birth   `kf3: chan 0x<client>:0x<handle> RC-UNARMED: …`   (chan.rs, three producers)
#   RC-NONE birth      `kf3: chan 0x<client>:0x<handle> RC-NONE: …`
#   C′ refused map     `kf3: mem t=<s>s REFUSED VasKey(<n>) root 0x…: <n> run(s) not applied: …`
#                      (device.rs `REFUSED {why}` over vasmgr.rs; any `REFUSED VasKey(` line counts)
#   named host line    `kf3: UNSERVICED-GPU-FAULT guest client 0x…`
#   RC posted          `kf3: RC_TRIGGERED posted: guest chid 0x…`
set -uo pipefail
RE_DEFECT='kf3: mem t=[0-9.]+s REFUSED (VasKey\(|.* run\(s\) not applied: )|kf3: chan 0x[0-9a-f]+:0x[0-9a-f]+ RC-UNARMED: '
RE_NONE='kf3: chan 0x[0-9a-f]+:0x[0-9a-f]+ RC-NONE: '
RE_UNSERVICED='kf3: UNSERVICED-GPU-FAULT guest client 0x[0-9a-f]+ '
RE_POSTED='kf3: RC_TRIGGERED posted: guest chid 0x[0-9a-f]+ '
# ★ The list, and the ONE sanctioned exception to "bare metal passes + guest fails ⇒ kayfabe bug"
# (OWNER_RULINGS §A.10): managed memory is not a release target (§I). Every other failing row stays
# a kayfabe bug.
EXPECTED_LOUD="UnifiedMemoryStreams UnifiedMemoryPerf conjugateGradientUM attach_verify um_cpuinit um_gpufirst um_pageable"
DISCARDS_STATUS="conjugateGradientUM"

app=${1:?app}; glog=${2:?guest.log}; gdm=${3:?guest_dmesg.log}; klog=${4:?kf3.log}; line=${5:?APPRES line}
# `why` carries no spaces: the hook appends it to a res line that is parsed as ` key=value` tokens
say(){ echo "LOUD=$1 why=$(tr ' ' '_' <<<"$2")"; exit 0; }
has(){ [ -r "$2" ] && grep -aqE "$1" "$2"; }
# the FIRST `key=value` token before `note=` (a note is free text and may itself say `rc=0`)
field(){ awk -v k="$1" '{for(i=1;i<=NF;i++){if($i~/^note=/)exit; if(index($i,k"=")==1){print substr($i,length(k)+2);exit}}}' <<<"$line"; }

case " $EXPECTED_LOUD " in *" $app "*) ;; *) say - not-a-managed-memory-row ;; esac
v=$(field verdict); rc=$(field rc)
case "$v" in
  PASS) say PASS "the row passed" ;;
  NOTRUN|BOOT_FAIL) say UNTESTED "verdict=$v" ;;
esac
defect=$( [ -r "$klog" ] && grep -aE -m1 "$RE_DEFECT" "$klog" | cut -c1-120 | tr ' |' '_/' )
[ -n "$defect" ] && say KF3_DEFECT "kf3:$defect"
case "$v" in TIMEOUT|HANG|GUEST_DEAD) say SILENT "verdict=$v — a hang is never loud" ;; esac
miss=""
has 'NVRM: Xid \([^)]*\): 31, .*kayfabe:' "$gdm" || miss="$miss,no-guest-kayfabe-xid31"
has "$RE_UNSERVICED" "$klog" || miss="$miss,no-UNSERVICED-GPU-FAULT"
has "$RE_POSTED" "$klog" || miss="$miss,no-RC_TRIGGERED-posted"
has "$RE_NONE" "$klog" && miss="$miss,a-twin-with-no-notifier(RC-NONE)"
apperr=0
has 'code=7(00|19)|-> 7(00|19)|CUBLAS_STATUS_EXECUTION_FAILED' "$glog" && apperr=1
case "$rc" in ''|0|124|137|-|ssh*) ;; *[!0-9]*) ;; *) apperr=1 ;; esac
if [ $apperr = 0 ]; then
  case " $DISCARDS_STATUS " in *" $app "*) ;; *) miss="$miss,the-app-saw-no-error(rc=$rc)" ;; esac
fi
[ -z "$miss" ] && say EXPECTED_LOUD "guest-xid31-kayfabe+UNSERVICED+RC_TRIGGERED+app-error(rc=$rc)"
say SILENT "${miss#,}"
