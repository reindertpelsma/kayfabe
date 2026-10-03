#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# boot_gate.sh <run_<tag>_qemu.log | boot_<tag>.qemu.log.zst> — the apps lane's per-boot silent-twin
# gate (release item §I, docs/design/V3_APP_MATRIX.md §R5.3; design §3.5 "Every apps boot asserts
# rc[unarmed=0 none=0]").
#
# A host twin whose error notifier could not be armed (`RC-UNARMED`) or whose guest declared none
# (`RC-NONE`) turns a GPU fault into a SILENT HANG: the host writes no RC record the guest can see,
# so kf3 posts no RC_TRIGGERED and no Xid. Such a twin can sit under ANY row — not only the
# managed-memory rows loud_verdict.sh scores — so the gate is per BOOT, over the whole kf3 log.
#
# Prints exactly one line, never exits nonzero on a verdict:
#   GATE=PASS|FAIL|UNMEASURED unarmed=<n|-> none=<n|-> births=<n> why=<reason-without-spaces>
#   PASS        the LAST complete `rc[armed=N unarmed=U none=M` status counter in the log has U=0 and
#               M=0, AND the log holds no `kf3: chan 0x…:0x… RC-UNARMED:` / `RC-NONE:` birth line
#   FAIL        either count is nonzero, or a birth line is present (the birth lines are printed
#               synchronously at birth; the status line only every 2 s on change, so both are read)
#   UNMEASURED  no readable log, or no status line carries `none=` (a kf3 older than 2026-10-03, or a
#               boot that died before its first status line). ⊘ NEVER a pass: a gate that cannot read
#               its counter has not asserted anything.
# ⚠ Keyed on line SHAPES, never bare words: kf3's boot-report sentence (DELIVERY_UNBUILT) names
# `RC-UNARMED` and `rc[unarmed=], rc[none=]` in prose (loud_verdict.sh's ⊘ CORRECTED note).
set -uo pipefail
log=${1:?qemu.log}
RE_BIRTH='kf3: chan 0x[0-9a-f]+:0x[0-9a-f]+ RC-(UNARMED|NONE): '
RE_STATUS='rc\[armed=[0-9]+ unarmed=[0-9]+ none=[0-9]+'
say(){ echo "GATE=$1 unarmed=$2 none=$3 births=$4 why=$(tr ' ' '_' <<<"$5")"; exit 0; }
[ -r "$log" ] || say UNMEASURED - - 0 "no readable kf3 log at $log"
T=$(mktemp); trap 'rm -f "$T"' EXIT
case "$log" in
  *.zst) zstd -dcq "$log" > "$T" 2>/dev/null || say UNMEASURED - - 0 "cannot decompress $log" ;;
  *) cat "$log" > "$T" ;;
esac
births=$(grep -acE "$RE_BIRTH" "$T"); births=${births:-0}
st=$(grep -aoE "$RE_STATUS" "$T" | tail -1)
if [ -z "$st" ]; then
  w="no rc[armed= unarmed= none=] status line (kf3 older than 2026-10-03, or no status printed)"
  [ "$births" -gt 0 ] && say FAIL - - "$births" "$births silent-twin birth line(s); $w"
  say UNMEASURED - - "$births" "$w"
fi
u=$(sed -n 's/.* unarmed=\([0-9]*\).*/\1/p' <<<"$st"); n=$(sed -n 's/.* none=\([0-9]*\).*/\1/p' <<<"$st")
first=$(grep -aoE -m1 "$RE_BIRTH" "$T")
if [ "$u" != 0 ] || [ "$n" != 0 ] || [ "$births" -gt 0 ]; then
  say FAIL "$u" "$n" "$births" "silent twins: rc[unarmed=$u none=$n], $births birth line(s)${first:+, first: $first}"
fi
say PASS 0 0 0 "rc[unarmed=0 none=0] and no silent-twin birth line"
