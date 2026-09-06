#!/usr/bin/env bash
# ★★★★★ w377 — the LATE-MAP RACE, run INSIDE the Mode-2 guest by a RAW CLIENT.
#
# The probe is two FIFO semaphore methods and no libcuda at all:
#     [ SEM_ACQUIRE(fence_va,  FENCE_VAL) ]   <- the engine stalls here
#     [ SEM_RELEASE(target_va, MAGIC)     ]   <- touches the LATE-mapped page
# ARM A maps the target BEFORE the doorbell (positive control).
# ARM B maps it AFTER  the doorbell, then releases the fence from the CPU (the race).
#
# ⚠ A guest user process gets a USER RM client, and `Boundary::channel_kind`
# (`kayfabe-core/src/project.rs:311`) makes every non-SYSTEM-anchored channel
# `GuestChannelKind::Passthrough`. So this exercises the passthrough path BY CONSTRUCTION,
# with no CUDA runtime anywhere in the picture.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#   (A) A PASS, B PASS  => the late mapping REACHED a running channel under kayfabe.
#                          The hazard is covered here; escalate to the wider arms (C/D/E).
#   (B) A PASS, B FAIL  => ★★★★★ THE REPRO. A red we own, ~30 lines, no proprietary
#                          runtime, deterministic. THIS IS THE DELIVERABLE, not a defeat:
#                          it is the thing the architecture gets iterated against.
#                          ⚠ Only a FINDING if the NATIVE arm B passed — see w377_racemap.sh.
#   (C) A PASS, B "RACE NOT RUN" => the acquire never blocked. The PROBE is wrong, not the
#                          system. Do not report it as either colour.
#   (D) A FAIL          => ⊘ B IS UNINTERPRETABLE. The positive control failed, so the
#                          submit path itself is broken and B measures that, not the race.
#   (E) no verdict line => ⊘ UNMEASURED. It is NOT a failure value. Say where it stopped.
#
# ⊘ `dlen=0` reasoning applies to this file too: an empty probe log is a state that needs
#   its own check, not an absence of findings.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
SCP_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5)
BIN=${RACEMAP_BIN:-}
TMO=${RACEMAP_TIMEOUT:-300}

echo "=== ★★★★★ w377 LATE-MAP RACE — RAW CLIENT, PASSTHROUGH CHANNEL, NO libcuda ==="

if ! $G true >/dev/null 2>&1; then
  echo "RACEMAP_OUTCOME=(E) UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi

# --- ship the raw client in ------------------------------------------------------------
# ⚠ Located, never assumed: a missing binary must attribute to the BUILD, not to the GPU.
if [ -z "$BIN" ]; then
  for c in "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}"/x86_64-unknown-linux-musl/release/rmladder \
           "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}"/release/rmladder \
           "${KAYFABE_REPO:-/root/kayfabe}"/target/x86_64-unknown-linux-musl/release/rmladder; do
    [ -x "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  echo "RACEMAP_BIN_FOUND=no"
  echo "RACEMAP_OUTCOME=(E) UNMEASURED_NO_BINARY — rmladder was not built for the guest"
  exit 0
fi
echo "RACEMAP_BIN_FOUND=yes ($BIN, $(stat -c %s "$BIN" 2>/dev/null) bytes)"
if ! scp "${SCP_OPTS[@]}" "$BIN" ubuntu@192.168.77.2:/tmp/rmladder >/dev/null 2>&1; then
  echo "RACEMAP_OUTCOME=(E) UNMEASURED_SCP_FAILED"; exit 0
fi
$G 'chmod +x /tmp/rmladder' >/dev/null 2>&1

echo "--- guest preconditions ---"
echo "GUEST_NVIDIA_NODES=$($G 'ls /dev/nvidia* 2>&1 | tr "\n" " "' 2>&1 | tr -d '\r')"
echo "GUEST_DRIVER=$($G 'cat /proc/driver/nvidia/version 2>&1 | head -1' 2>&1 | tr -d '\r')"
$G 'sudo dmesg -c >/dev/null 2>&1' >/dev/null 2>&1

# --- run -------------------------------------------------------------------------------
echo "--- rmladder --late-map-race (in guest) ---"
OUT=$($G "timeout $TMO /tmp/rmladder --late-map-race 2>&1" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'

echo "--- guest dmesg (a GUEST-visible fault lands here, not in the host log) ---"
$G 'sudo dmesg 2>&1 | grep -iE "xid|nvrm|fault" | tail -20' 2>&1 | tr -d '\r' | sed 's/^/    /'

# --- grade -----------------------------------------------------------------------------
# ⊘ Grade on the LINES, never on the exit status: a timeout kills the process and its
#   status says nothing about how far the probe got.
ARM_A=$(echo "$OUT" | sed -n 's/^RACEMAP_ARM_A=//p' | tail -1)
ARM_B=$(echo "$OUT" | sed -n 's/^RACEMAP_ARM_B=//p' | tail -1)
echo ""
echo "GUEST_ARM_A=${ARM_A:-NONE}"
echo "GUEST_ARM_B=${ARM_B:-NONE}"
echo "=== ★★★★★ THE GUEST VERDICT, stated once, in the pre-registered vocabulary"
if [ -z "$ARM_A" ] && [ -z "$ARM_B" ]; then
  echo "    RACEMAP_OUTCOME=(E) ⊘ UNMEASURED — no RACEMAP_ARM_ line at all. NOT a failure value."
elif [ "$ARM_A" != PASS ]; then
  echo "    RACEMAP_OUTCOME=(D) ⊘ ARM B UNINTERPRETABLE — positive control A=$ARM_A."
  echo "        The submit path itself is broken; B measures that, not the race."
elif [ "$ARM_B" = "NOTRUN" ]; then
  echo "    RACEMAP_OUTCOME=(C) ?? RACE NOT RUN — the acquire never blocked. The PROBE is wrong."
elif [ "$ARM_B" = PASS ]; then
  echo "    RACEMAP_OUTCOME=(A) the late mapping REACHED a running channel under kayfabe."
  echo "        ⊘ Covered HERE only; escalate to the wider arms before calling the class closed."
else
  echo "    RACEMAP_OUTCOME=(B) ★★★★★ THE REPRO — A=PASS B=$ARM_B, raw client, no libcuda."
  echo "        ⚠ A FINDING only if the NATIVE arm B passed. If native B also fails, the"
  echo "        driver does not guarantee this and our red is PARITY, not a defect."
fi
