#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# collect.sh <box-ssh-alias> <out-dir> — pull one box's die-sweep TEXT evidence into the repo (boxes are
# untrusted; only text comes back, never a binary; a secrets directory is never read).
# Run on the relay (the dev machine), from a checkout. Re-runnable (overwrites).
set -uo pipefail
BOX=${1:?box alias}; OUT=${2:?out dir}; W=${WIN_DIR:-/root/win}; R=${WIN_RUN:-$W/run}
mkdir -p "$OUT"
get() { ssh -o BatchMode=yes "$BOX" "$1" 2> /dev/null | tr -d '\r' > "$OUT/$2"; }
# the box's own provenance lines (no ssh keys, no addresses, no secrets: prov.log carries rc lines only)
get 'grep -aE "_RC=|READY|REBOOT|^EXIT|^START|^[0-9a-f]{9} " /root/prov/prov.log' prov_summary.txt
get 'cat /root/prov/v3_gates.log 2>/dev/null | grep -aE "V3_GATES|VERDICT|HEAD=|GPU="' v3_gates_summary.txt
get 'cat /root/box_facts_before.txt /root/box_facts_after.txt 2>/dev/null' box_facts.txt
for p in phase1 bind probe phase2; do
    # drop the PowerShell CLIXML progress noise; keep the evidence lines
    get "[ -f $R/$p.log ] && grep -av '^<Objs\|^#< CLIXML' $R/$p.log | cut -c1-400" "$p.log"
    [ -s "$OUT/$p.log" ] || rm -f "$OUT/$p.log"
done
get "cat $R/marker.txt $R/flags.txt 2>/dev/null" run_marker_flags.txt
# kf3's own log, filtered: counters, refusals, resets, display, errors (the raw log is 100+ MB of counter lines)
get "ls -1 $R/qemu*.log 2>/dev/null | while read f; do echo \"== \$f\"; grep -aE 'kf3: (display|realize|RESET|reset|refus|FATAL|error|WARN)|REFUSED|STOPPED|TDR|Running -> Suspending|unload|DEVICE RESET|panic' \"\$f\" | cut -c1-300 | awk '{c[\$0]++; if (c[\$0]<=3) print}' | grep -av 'display fps\\[' | head -300; echo '-- last counter line'; grep -a 'kf3: family=' \"\$f\" | tail -1 | cut -c1-1200; done" qemu_filtered.txt
# the owner host's own counters (/data/claude-cnt.sh), per qemu log: a reset or TDR cycle is "Running -> Suspending"
get "for f in $R/qemu*.log; do echo \"== \$f\"; for k in 'GSP phase Running -> Suspending' 'USER_RING_REFUSED' 'USER_RING_SCAN' 'REFUSED fn76' 'display: STALL' 'BLACK:' 'HOST-ABI REFUSED'; do echo \"\$k: \$(grep -ac \"\$k\" \$f)\"; done; done" counts.txt
get "dmesg | grep -aE 'NVRM|Xid' | tail -40" host_dmesg_nvrm.txt
echo "COLLECT_DONE $OUT $(ls "$OUT" | wc -l) files"
