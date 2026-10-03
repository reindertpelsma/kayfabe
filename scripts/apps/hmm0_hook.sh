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
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G=${APPS_GSSH:-$HERE/../bench/gssh_nv}
O=${APPS_RESULTS:-/workspace/apps/results}/r5hmm0.txt; mkdir -p "$(dirname "$O")"
{
  echo "HMM0_START tag=$TAG rev=$(git -C "$HERE/../.." rev-parse --short=8 HEAD 2>/dev/null) at=$(date -Is)"
  # um_probe is a host-built probe newer than the guest image: push bin/ as apps_hook.sh does
  tar -C /workspace/apps/bundle -cf - bin | $G 'sudo tar -C /opt/apps/bundle -xf -'
  $G 'sudo modprobe -r nvidia_uvm; sudo modprobe nvidia_uvm uvm_disable_hmm=1; echo "hmm_disabled=$(cat /sys/module/nvidia_uvm/parameters/uvm_disable_hmm)"; sudo /opt/apps/bundle/bin/um_probe attrs; sudo timeout 60 /opt/apps/bundle/bin/um_probe pageable; echo "pageable_rc=$?"; sudo dmesg | grep "NVRM: Xid" | tail -3'
  echo "HMM0_END rc=$?"
} 2>&1 | tr -d '\r' | tee "$O"
