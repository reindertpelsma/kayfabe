#!/usr/bin/env bash
# ★ v3 harness gates — build and run every `kf-gate*` on THIS box's GPU, no QEMU, no guest.
#
# Usage (on the box, in the kayfabe checkout):  bash scripts/bench/v3_gates.sh [out.log]
#
# ⊘ Three rules this script exists to encode, each paid for:
# - **The revision is part of the result** (CLAUDE.md: a bench claim without its source revision
#   is not a claim). HEAD and a dirty flag are the first lines written.
# - **A start marker and an exit line**, so "file exists but has no terminator" is detectable — a
#   killed job and a running one otherwise look identical.
# - **The verdict is each gate's own `GATEn_VERDICT=` line**, never "the binary exited" or "we got to
#   the end" (the_last_line_is_not_the_verdict). A gate that prints no verdict line is a FAIL.
# - **Gate 10 (2026-10-10, D3)** is `kf-micro-reserve-probe reserve`: `GATE10_VERDICT=PASS|FALLBACK|FAIL`
#   (FALLBACK = host RM refused reservations, the 4 KiB floor is active: loud, not a failure), and
#   `gate10=` is a field of `V3_GATES_SUMMARY`. The "9/9" count (`pass=`) is the nine only.
# - **Every channel the gates birth is USER** (THE_CONSTRAINTS §30): each `kf-host: channel birth`
#   line must read `PRIVILEGED_CHANNEL=0 privilege=USER`, no refusal line may appear, and a run with
#   no birth line at all fails (`V3_GATES_BIRTHS … ok=0`, exit 1).
set -uo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:-/root/prov/v3_gates.log}
TARGET=${CARGO_TARGET_DIR:-target}
export PATH="$PATH:$HOME/.cargo/bin"
{
  echo "V3_GATES_START $(date -Is)"
  echo "HEAD=$(git rev-parse --short=8 HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
  nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1 | sed 's/^/GPU=/'
  if ! cargo build -q --release -p kf-harness --bins 2>&1 | tail -20; then
    echo "BUILD=FAIL"; echo "V3_GATES_EXIT pass=0 fail=all $(date -Is)"; exit 1
  fi
  pass=0; fail=0; failed=""; births=0; user=0; refused=0
  # Explicit census: a missing executable is a failure, never a zero-gate success.
  # Respect the same target directory cargo just built into.
  for n in {1..9}; do
    bin="$TARGET/release/kf-gate$n"
    g=$(basename "$bin")
    echo "=== $g"
    if [ ! -x "$bin" ]; then
      echo "MISSING_GATE=$g"; fail=$((fail+1)); failed="$failed $g(missing)"; continue
    fi
    out=$(timeout 180 "$bin" 2>&1); rc=$?
    echo "$out"
    # ★ THE_CONSTRAINTS §30: every channel a gate births through kf-host must read USER.
    births=$((births + $(echo "$out" | grep -ac 'kf-host: channel birth ')))
    user=$((user + $(echo "$out" | grep -a 'kf-host: channel birth ' | grep -ac ' PRIVILEGED_CHANNEL=0 privilege=USER ')))
    refused=$((refused + $(echo "$out" | grep -acE 'PRIVILEGED CHANNEL REFUSED|CHANNEL BIRTH REFUSED|CHANNEL CLASS REFUSED|CUDA THREAD REFUSED')))
    v=$(echo "$out" | grep -E '^GATE[0-9]+_VERDICT=' | tail -1 | cut -d= -f2)
    if [ "$v" = "PASS" ] && [ "$rc" -eq 0 ]; then pass=$((pass+1)); else fail=$((fail+1)); failed="$failed $g(rc=$rc,verdict=${v:-NONE})"; fi
  done
  # ★ D3 (2026-10-10) — GATE 10, the micro-reservation probe (`V3_BATCHED_MAP.md` §8.8.3). Micro
  # reservations are the default for the batched map, so the claim "host RM accepts a small FIXED
  # reservation in the unreserved range and unmaps part of what is mapped through it exactly" is
  # re-measured on every box. It is NOT one of the nine `kf-gate*` binaries and is NOT in the
  # `pass=` / `fail=` counts: the "9/9" keeps its meaning. Its verdict is the probe's own
  # `MICRO_RESERVE_VERDICT arm=reserve PASS|FALLBACK|FAIL`, not its exit status alone:
  #   PASS      host RM accepted the reservation and every remnant read (the exact path works);
  #   FALLBACK  host RM REFUSED the reservation — NOT a failure: the batched map falls back to the
  #             4 KiB grain (D3), so the driver stays usable. Printed loudly; the exit status stays 0;
  #   FAIL      anything else — above all an accepted reservation whose remnant does not read (an
  #             inconsistency: the exact-partial-unmap premise is wrong), a probe error, a missing
  #             binary, no verdict line.
  # The probe's channel births join the USER census below. Informational lines
  # (`reserve_flat_fb_alias`, the census) are in the log and never gate.
  g10=FAIL
  bin10="$TARGET/release/kf-micro-reserve-probe"
  echo "=== kf-micro-reserve-probe reserve (gate 10)"
  if [ ! -x "$bin10" ]; then
    echo "MISSING_GATE=kf-micro-reserve-probe"
  else
    out10=$(KF3_WIN_USER_CHANNELS_PASSTHROUGH=1 timeout 180 "$bin10" reserve 2>&1); rc10=$?
    echo "$out10"
    births=$((births + $(echo "$out10" | grep -ac 'kf-host: channel birth ')))
    user=$((user + $(echo "$out10" | grep -a 'kf-host: channel birth ' | grep -ac ' PRIVILEGED_CHANNEL=0 privilege=USER ')))
    refused=$((refused + $(echo "$out10" | grep -acE 'PRIVILEGED CHANNEL REFUSED|CHANNEL BIRTH REFUSED|CHANNEL CLASS REFUSED|CUDA THREAD REFUSED')))
    v10=$(echo "$out10" | grep -E '^MICRO_RESERVE_VERDICT arm=reserve ' | tail -1 | awk '{print $3}')
    if [ "$rc10" -eq 0 ] && [ "$v10" = "PASS" ]; then g10=PASS; fi
    # FALLBACK (reservations refused) passes ONLY with the probe's own proof that the flat FB alias
    # still places in that configuration (the 6fafcc6e 0/30 failure class): a FALLBACK verdict
    # without the passing `fallback_flat_fb_alias_placeable` check line is a FAIL.
    if [ "$rc10" -eq 0 ] && [ "$v10" = "FALLBACK" ]; then
      if echo "$out10" | grep -aq '^CHECK fallback_flat_fb_alias_placeable PASS'; then
        g10=FALLBACK
      else
        echo "GATE10_FALLBACK_UNPROVEN: the probe says FALLBACK but did not prove the flat FB alias places without reservations — FAIL"
      fi
    fi
  fi
  echo "GATE10_VERDICT=$g10"
  if [ "$g10" = "FALLBACK" ]; then
    echo "GATE10_FALLBACK_ACTIVE: host RM refused ALL small micro reservations with a refusal status on this driver, and the probe proved a 2 MiB leaf reservation at the flat FB alias base still works — the batched map runs on the per-leaf / 4 KiB ladder (the probe proves the DRIVER can do it, not that kf-mem does: the model test and the fast suite cover that); not a failure, but NOT the default path"
  fi
  # Gate 10 is part of the summary line every consumer reads (sweep.sh, matrix_table.py, merge_check.sh).
  echo "V3_GATES_SUMMARY pass=$pass fail=$fail gate10=$g10${failed:+ failed:$failed}"
  # ⊘ A gate run with no channel birth at all has not shown the birth check can report one.
  births_ok=0
  [ "$births" -ge 1 ] && [ "$user" -eq "$births" ] && [ "$refused" -eq 0 ] && births_ok=1
  echo "V3_GATES_BIRTHS births=$births user=$user refused=$refused ok=$births_ok"
  echo "V3_GATES_EXIT pass=$pass fail=$fail births_ok=$births_ok gate10=$g10 $(date -Is)"
  [ "$pass" -eq 9 ] && [ "$fail" -eq 0 ] && [ "$births_ok" -eq 1 ] && { [ "$g10" = "PASS" ] || [ "$g10" = "FALLBACK" ]; }
} 2>&1 | tee "$OUT"
