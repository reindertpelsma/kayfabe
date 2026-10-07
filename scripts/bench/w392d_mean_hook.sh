#!/usr/bin/env bash
# ★★★★★ w392d — THE OWNER'S FALSIFIER: THE SAME CLIENT, IN THE GUEST.
#
# **Owner, 2026-09-08:** *"then it must pass on the guest if you have all maps"*.
#
# That is the whole rung, and it is a genuine falsifier rather than a demonstration:
#
#   the client PASSES ON BARE METAL  (measured, host of box 50260029, all five steps
#                                     rmStatus 0x0 — see w392_the_memop_transport_is_unreached.md §5b)
#   ⇒ every refusal it now collects inside the guest is OURS, by construction.
#
# ## ★★★ WHY THIS IS WORTH A BOOT — it asks TWO questions with one workload
#
#   Q1  Does our emulated device SERVE the UVM registration path at all?
#       `UVM_INITIALIZE` → `UVM_MM_INITIALIZE` → `UVM_REGISTER_GPU` → `UVM_REGISTER_GPU_VASPACE`.
#       A raw client, no libcuda, no CUDA runtime — so a failure names one ioctl instead of
#       dying somewhere inside a 500-ioctl `cuInit`.
#
#   Q2  Does point 2's TRANSPORT reach us? `UVM_REGISTER_GPU` is what makes nvidia-uvm build
#       its channel manager, whose `UVM_CHANNEL_TYPE_MEMOPS` pool carries every
#       `MMU_TLB_INVALIDATE` on this chip. So `MEMOP-CENSUS` in the QEMU log, read AFTER this
#       hook, is the first census of that transport driven by something AIMED at it rather
#       than by whatever a CUDA program happened to do.
#
# ⊘ **Q2 IS ONLY INTERPRETABLE IF Q1 PASSED.** If the registration is refused, no channel
#   manager exists, and a `seen=0` census afterwards is a fact about the refusal — not about
#   the transport. The verdict below refuses to grade Q2 in that case.
#
# ## ★★ PRE-REGISTERED OUTCOMES — stated before the boot, so none reads as the favourable one
#
#   (P) all four ioctls 0x0        => ★★★★★ THE FALSIFIER SURVIVES. Our device serves the
#                                     UVM path as the real driver does. Read the census.
#   (R) one refuses, named         => ★★★★★ A NAMED GAP, and the most valuable outcome
#                                     available: the client is known-good, so the refusing
#                                     ioctl IS the coverage hole. NOT a defeat.
#   (E) no verdict line at all     => ⊘ UNMEASURED. Not a failure value. Say where it stopped.
#   (N) no binary for the guest    => ⊘ UNMEASURED. The rung did not run.
#
# ⚠ **This client checks STATUSES, not CONTENT** (owner, same session). It cannot see the
#   corruption class w392 measured — 16 tokens of garbage with every `rmStatus` clean. A (P)
#   here is *"the registration path is served"*, NOT *"mappings are correct"*. The
#   pattern-write / churn / read-back-compare arm is owed and is NOT in this run.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
SCP_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5)
TMO=${UVM_TIMEOUT:-300}

echo "=== ★★★★★ w392d — THE UVM RAW CLIENT, IN THE GUEST (owner's falsifier) ==="

BIN=${UVM_BIN:-}
if [ -z "$BIN" ]; then
  for c in "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
           "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}"/release/kayfabe-rm-ladder \
           "${KAYFABE_REPO:-/root/kayfabe}"/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder; do
    [ -x "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ]; then
  echo "W392D_GUEST_OUTCOME=(N) ⊘ UNMEASURED_NO_BINARY — rmladder was not built for the guest"
  exit 0
fi
echo "info  W392D bin           = $BIN"

if ! $G true >/dev/null 2>&1; then
  echo "W392D_GUEST_OUTCOME=(E) ⊘ UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi
if ! scp "${SCP_OPTS[@]}" "$BIN" ubuntu@192.168.77.2:/tmp/rmladder >/dev/null 2>&1; then
  echo "W392D_GUEST_OUTCOME=(N) ⊘ UNMEASURED — could not copy the binary into the guest"; exit 0
fi
$G 'chmod +x /tmp/rmladder' >/dev/null 2>&1

echo "--- guest preconditions (a missing node is NOT a UVM result) ---"
$G 'ls -la /dev/nvidia-uvm 2>&1; lsmod | grep -c nvidia_uvm' 2>&1 | sed 's/^/    /'

echo "--- rmladder --uvm-mean (FULL: threads/rounds above) --mean-falsify (in guest) ---"
# ★★★ THE FULL MEAN TEST, not the smoke configuration — owner, 2026-09-10:
# *"raw client with the full mean teast, not just the simple one."*
#
# The client's own defaults are `threads 4 / p1_rounds 4`, and every run this campaign has
# graded used them by omission. They are the SMOKE shape: four threads is not a concurrency
# test on a box with more cores than that, and four rounds barely exercises the stale-mapping
# path the rounds exist for (`--mean-rounds` refuses < 2 by name, *"cannot see a stale
# mapping"*, so rounds are the axis that makes the test mean anything).
#
# ⊘ Overridable, but the DEFAULT here is the full shape — a knob whose default is the weak
# setting is a knob that measures the weak setting forever, which is the leg-8 lesson.
MEAN_THREADS=${MEAN_THREADS:-8}
MEAN_ROUNDS=${MEAN_ROUNDS:-8}
echo "    W392D_MEAN_CONFIG=threads:$MEAN_THREADS rounds:$MEAN_ROUNDS falsify:on (⊘ client defaults are 4/4)"
OUT=$($G "echo W392D_GUEST_STARTED=\$(date -u +%FT%TZ); sudo timeout $TMO /tmp/rmladder --gpu 0 --uvm-mean --mean-threads $MEAN_THREADS --mean-rounds $MEAN_ROUNDS --mean-falsify 2>&1; echo W392D_GUEST_RC=\$?" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'

pick() { echo "$OUT" | sed -n "s/.*W392D $1 *= *//p" | tail -1; }
INIT=$(pick INITIALIZE); MM=$(pick MM_INITIALIZE); REG=$(pick REGISTER_GPU); VAS=$(pick REGISTER_VAS)
GRC=$(echo "$OUT" | sed -n 's/^W392D_GUEST_RC=//p' | tail -1)

echo ""
echo "W392D_GUEST_INITIALIZE=${INIT:-ABSENT}"
echo "W392D_GUEST_MM_INITIALIZE=${MM:-ABSENT}"
echo "W392D_GUEST_REGISTER_GPU=${REG:-ABSENT}"
echo "W392D_GUEST_REGISTER_VAS=${VAS:-ABSENT}"
echo "W392D_GUEST_RC=${GRC:-ABSENT}"

echo "W392D_GUEST_LEDGER:"; echo "$OUT" | grep -aE "✔ VERIFIED|⊘ UNEXERCISED|CONTENT MISMATCH|REFUSED at|THREADS |MEAN_FALSIFIER" | sed 's/^/    /'
echo "=== ★★★★★ THE VERDICT — pre-registered, stated once ==="
if ! echo "$OUT" | grep -q "W392D config"; then
  echo "    W392D_GUEST_OUTCOME=(E) ⊘ UNMEASURED — no W392D line at all. The client did not"
  echo "        reach its first ioctl. Read the transcript above; this is NOT a refusal."
elif echo "$OUT" | grep -q "W392D_OUTCOME=(P)"; then
  echo "    W392D_GUEST_OUTCOME=(P) ★★★★★ THE FALSIFIER SURVIVES — the SAME client that"
  echo "        passes on bare metal passes in the guest. Our device serves the UVM"
  echo "        registration path, so nvidia-uvm built its channel manager here too."
  echo "        ⇒ NOW read MEMOP-CENSUS in the QEMU log. It is interpretable for the first"
  echo "          time: the transport it counts is known to EXIST on this boot."
  echo "        ⊘ This says NOTHING about whether mappings are CORRECT — statuses only."
else
  echo "    W392D_GUEST_OUTCOME=(R) ★★★★★ A NAMED GAP — and this is the most valuable"
  echo "        outcome available, not a defeat. The client is KNOWN-GOOD on bare metal, so"
  echo "        the first non-zero above IS the coverage hole, named to one ioctl."
  echo "        ⊘ And a MEMOP-CENSUS of zero after this is a fact about THIS refusal, not"
  echo "          about the transport — do not read it as either."
fi
echo "=== w392d guest hook DONE ==="
