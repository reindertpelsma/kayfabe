#!/usr/bin/env bash
# ★★★★★ w392c — THE OWNER'S FALSIFIER: THE SAME CLIENT, IN THE GUEST.
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

echo "=== ★★★★★ w392c — THE UVM RAW CLIENT, IN THE GUEST (owner's falsifier) ==="

BIN=${UVM_BIN:-}
if [ -z "$BIN" ]; then
  for c in "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
           "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}"/release/kayfabe-rm-ladder \
           "${KAYFABE_REPO:-/root/kayfabe}"/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder; do
    [ -x "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ]; then
  echo "W392C_GUEST_OUTCOME=(N) ⊘ UNMEASURED_NO_BINARY — rmladder was not built for the guest"
  exit 0
fi
echo "info  W392C bin           = $BIN"

if ! $G true >/dev/null 2>&1; then
  echo "W392C_GUEST_OUTCOME=(E) ⊘ UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi
if ! scp "${SCP_OPTS[@]}" "$BIN" ubuntu@192.168.77.2:/tmp/rmladder >/dev/null 2>&1; then
  echo "W392C_GUEST_OUTCOME=(N) ⊘ UNMEASURED — could not copy the binary into the guest"; exit 0
fi
$G 'chmod +x /tmp/rmladder' >/dev/null 2>&1

echo "--- guest preconditions (a missing node is NOT a UVM result) ---"
$G 'ls -la /dev/nvidia-uvm 2>&1; lsmod | grep -c nvidia_uvm' 2>&1 | sed 's/^/    /'

echo "--- rmladder --uvm-invalidate (in guest) ---"
OUT=$($G "echo W392C_GUEST_STARTED=\$(date -u +%FT%TZ); sudo timeout $TMO /tmp/rmladder --gpu 0 --uvm-invalidate 2>&1; echo W392C_GUEST_RC=\$?" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'

pick() { echo "$OUT" | sed -n "s/.*W392C $1 *= *//p" | tail -1; }
INIT=$(pick INITIALIZE); MM=$(pick MM_INITIALIZE); REG=$(pick REGISTER_GPU); VAS=$(pick REGISTER_VAS)
GRC=$(echo "$OUT" | sed -n 's/^W392C_GUEST_RC=//p' | tail -1)

echo ""
echo "W392C_GUEST_INITIALIZE=${INIT:-ABSENT}"
echo "W392C_GUEST_MM_INITIALIZE=${MM:-ABSENT}"
echo "W392C_GUEST_REGISTER_GPU=${REG:-ABSENT}"
echo "W392C_GUEST_REGISTER_VAS=${VAS:-ABSENT}"
echo "W392C_GUEST_RC=${GRC:-ABSENT}"

echo "=== ★★★★★ THE VERDICT — pre-registered, stated once ==="
if [ -z "${INIT:-}" ]; then
  echo "    W392C_GUEST_OUTCOME=(E) ⊘ UNMEASURED — no W392C line at all. The client did not"
  echo "        reach its first ioctl. Read the transcript above; this is NOT a refusal."
elif echo "$OUT" | grep -q "W392C_OUTCOME=(P)"; then
  echo "    W392C_GUEST_OUTCOME=(P) ★★★★★ THE FALSIFIER SURVIVES — the SAME client that"
  echo "        passes on bare metal passes in the guest. Our device serves the UVM"
  echo "        registration path, so nvidia-uvm built its channel manager here too."
  echo "        ⇒ NOW read MEMOP-CENSUS in the QEMU log. It is interpretable for the first"
  echo "          time: the transport it counts is known to EXIST on this boot."
  echo "        ⊘ This says NOTHING about whether mappings are CORRECT — statuses only."
else
  echo "    W392C_GUEST_OUTCOME=(R) ★★★★★ A NAMED GAP — and this is the most valuable"
  echo "        outcome available, not a defeat. The client is KNOWN-GOOD on bare metal, so"
  echo "        the first non-zero above IS the coverage hole, named to one ioctl."
  echo "        ⊘ And a MEMOP-CENSUS of zero after this is a fact about THIS refusal, not"
  echo "          about the transport — do not read it as either."
fi
echo "=== w392c guest hook DONE ==="
