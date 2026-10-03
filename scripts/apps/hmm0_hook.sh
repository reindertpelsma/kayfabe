#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# POST_CAPTURE_HOOK for boot_capture.sh: the kf3 guest with HMM OFF (release item §I, design §6
# "the guest with uvm_disable_hmm=1: ATTR line"; docs/design/V3_APP_MATRIX.md §R5.7). It answers the
# owner question whether release guidance may mention `options nvidia-uvm uvm_disable_hmm=1`:
#   - does the guest's nvidia-uvm take the option (`hmm_disabled=Y`)?
#   - what libcuda then reports (`um_probe attrs`: the ATTR line; bare metal with HMM off reads
#     pageableMemoryAccess=0, traces/v3_uvm_research/bm_rtx3060ti_580.159.04_hmm0.out:8)?
#   - does a pageable access still fail, and loudly (`um_probe pageable`: `-> 719`, a guest Xid)?
# Writes $APPS_RESULTS/r5hmm0.txt (default /workspace/apps/results), with a start marker and an
# exit line, so an empty or cut file is distinguishable from a finished one.
#
# ⊘ CORRECTED 2026-10-03 (review of 9390f51c): the exit line was `HMM0_END rc=$?` — the status of the
# remote command's LAST stage, `… | tail -3`, which is 0 whether the reload failed, um_probe hung or
# nothing matched; a terminator that read like a verdict. Every step now reports its OWN status by
# name, and the exit line carries the one verdict that matters — whether the ATTR line was measured
# with HMM really off:
#   HMM0_END status=MEASURED|UNMEASURED ssh_rc= reload_rc= hmm_disabled= attrs_rc= attr_lines=
#            pageable_rc= new_xid= why=
#   MEASURED    the module reloaded (reload_rc=0), took the option (hmm_disabled=Y), and
#               `um_probe attrs` exited 0 with an `ATTR ` line. Anything else is UNMEASURED: the ATTR
#               line, if any, is then NOT the HMM-off answer.
#   pageable_rc / new_xid   the pageable access's own exit status (124 = timed out, i.e. a hang) and
#               the guest Xid lines it added — read beside the verdict, never folded into it.
# `APPS_GSSH` replaces the guest ssh (test_hook.sh drives this hook offline with it).
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G=${APPS_GSSH:-$HERE/../bench/gssh_nv}
O=${APPS_RESULTS:-/workspace/apps/results}/r5hmm0.txt; mkdir -p "$(dirname "$O")"
# the guest side: each step's status captured by name, never a pipeline's last stage
# shellcheck disable=SC2016  # expanded on the guest
REMOTE='rl=0; sudo modprobe -r nvidia_uvm || rl=$?
if [ "$rl" = 0 ]; then sudo modprobe nvidia_uvm uvm_disable_hmm=1 || rl=$?; fi
echo "reload_rc=$rl"
echo "hmm_disabled=$(cat /sys/module/nvidia_uvm/parameters/uvm_disable_hmm 2>/dev/null || echo absent)"
sudo /opt/apps/bundle/bin/um_probe attrs; echo "attrs_rc=$?"
x0=$(sudo dmesg | grep -c "NVRM: Xid")
sudo timeout 60 /opt/apps/bundle/bin/um_probe pageable; echo "pageable_rc=$?"
xl=$(sudo dmesg | grep "NVRM: Xid" | tail -n +$((x0+1)))
[ -n "$xl" ] && echo "$xl"
echo "new_xid=$(printf "%s" "$xl" | grep -c .)"'
{
  echo "HMM0_START tag=$TAG rev=$(git -C "$HERE/../.." rev-parse --short=8 HEAD 2>/dev/null) at=$(date -Is)"
  # um_probe is a host-built probe newer than the guest image: push bin/ as apps_hook.sh does
  tar -C /workspace/apps/bundle -cf - bin | $G 'sudo tar -C /opt/apps/bundle -xf -'
  out=$($G "$REMOTE" 2>&1); src=$?
  out=${out//$'\r'/}
  echo "$out"
  kv(){ sed -n "s/^$1=//p" <<<"$out" | tail -1; }
  rl=$(kv reload_rc); hd=$(kv hmm_disabled); ar=$(kv attrs_rc); pr=$(kv pageable_rc); nx=$(kv new_xid)
  na=$(grep -c '^ATTR ' <<<"$out")
  why=""
  [ "$src" = 0 ] || why="$why,ssh_rc=$src"
  [ "$rl" = 0 ] || why="$why,reload_rc=${rl:-none}"
  [ "$hd" = Y ] || why="$why,hmm_disabled=${hd:-none}"
  { [ "$ar" = 0 ] && [ "$na" -gt 0 ]; } || why="$why,no-ATTR-line(attrs_rc=${ar:-none})"
  st=MEASURED; [ -n "$why" ] && st=UNMEASURED
  echo "HMM0_END status=$st ssh_rc=$src reload_rc=${rl:--} hmm_disabled=${hd:--} attrs_rc=${ar:--} attr_lines=$na pageable_rc=${pr:--} new_xid=${nx:--} why=${why#,}"
} 2>&1 | tee "$O"
