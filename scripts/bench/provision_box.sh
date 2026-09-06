#!/usr/bin/env bash
# ★★★ PROVISION A FRESH BENCH BOX — the recipe, so nobody rebuilds one from memory.
#
# Run `host_preflight.sh` FIRST. If it says HOST_FAULT, destroy the instance and rent
# another; provisioning a broken box wastes the time the preflight exists to save.
#
# ⚠ THE ONE STEP THAT IS NOT OBVIOUS AND FAILS LOUDLY-BUT-MISLEADINGLY:
#   `rustup target add x86_64-unknown-linux-musl`
# `kayfabe-isolate-host/build.rs` builds the embedded isolate as a **static musl binary**
# (`:164`), so without the musl std the WHOLE WORKSPACE fails with a bare
#   error[E0463]: can't find crate for `std`
# on `kayfabe-util` — a crate that has nothing to do with musl and names no target. The
# build.rs says so itself at `:41-44` ("every CI job simply declares the musl target
# alongside its own"); the error surfaces far from the cause.
set -euo pipefail
echo "PROVISION_START $(date -Is)"

export DEBIAN_FRONTEND=noninteractive

# ★★★★ DO THIS FIRST, BEFORE ANYTHING ELSE, AND UNDERSTAND WHY IT IS USUALLY TOO LATE.
# A freshly rented box starts `unattended-upgrade` AT BOOT. By the time you can ssh in, it
# is already running -- so masking the timers (below) prevents the NEXT run and does nothing
# about the one in flight. Measured 2026-09-06 on vast 50013922: it held the dpkg lock for
# 20+ minutes and its upgrade set included
#     linux-generic-hwe-22.04  linux-image-generic-hwe-22.04  linux-headers-generic-hwe-22.04
# i.e. A NEW KERNEL (6.8.0-59 running, 6.8.0-138 installed underneath it).
#
# ⚠ THE LATENT FAILURE THIS CAUSES, which is much worse than the delay:
# if you wait out the lock and then install the NVIDIA driver, DKMS builds against the
# RUNNING kernel. That module is correct, `/proc/driver/nvidia/version` reads right, every
# check passes -- and it DISAPPEARS at the next reboot, because the box comes up on the new
# kernel with no module built for it. The failure surfaces hours later, detached from its
# cause, as "the driver is just gone".
# ⇒ On a fresh box: mask the timers, WAIT for any in-flight run, REBOOT onto the new kernel,
#   and only then install the driver. `reboot_chain` in the bench notes does this.
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer >/dev/null 2>&1 && \
  echo "apt timers masked (prevents the NEXT run; see header re: the one already running)"
if pgrep -x unattended-upgr >/dev/null 2>&1 || fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; then
  echo "⚠ an unattended upgrade is ALREADY IN FLIGHT -- check whether it upgrades the kernel:"
  grep -oE "linux-(image|headers|generic)[a-z0-9.-]*" /var/log/unattended-upgrades/unattended-upgrades.log 2>/dev/null | sort -u | head
  echo "  if it does, you must REBOOT before installing the NVIDIA driver."
fi

# ⚠ A FRESH CLOUD BOX RUNS `unattended-upgrades` AT BOOT and it holds the dpkg lock.
# Measured on vast instance 50013922, 2026-09-06: provisioning launched ~7 minutes after
# first boot and died instantly with
#   E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 7603
# `apt-get` exits 100 without installing anything. ⇒ The failure is a RACE WITH THE BOX,
# not with our code, and retrying by hand a minute later "fixes" it — which is exactly why
# it never gets written down and bites the next person instead. Wait for the lock.
wait_for_dpkg() {
  local waited=0
  while fuser /var/lib/dpkg/lock-frontend /var/lib/apt/lists/lock >/dev/null 2>&1; do
    [ "$waited" -ge 600 ] && { echo "DPKG_LOCK_TIMEOUT after ${waited}s"; return 1; }
    [ $((waited % 60)) -eq 0 ] && echo "waiting for dpkg lock (${waited}s)"
    sleep 10; waited=$((waited + 10))
  done
  echo "dpkg lock free after ${waited}s"
}
# ★ ASK BEFORE WAITING. Measured on the same box: `unattended-upgrade` held the lock for
# over nine minutes, while EVERY package below was already installed -- the image ships
# them. Waiting for a lock to run an install that would be a no-op is pure dead time, and
# on a 600s ceiling it can fail the run outright. So probe first and only touch apt if
# something is genuinely missing.
NEED=""
for b in gcc pkg-config git curl clang lld python3; do
  command -v "$b" >/dev/null 2>&1 || NEED="$NEED $b"
done
[ -e /usr/include/openssl/ssl.h ] || NEED="$NEED libssl-dev"
if [ -n "$NEED" ]; then
  echo "apt needed for:$NEED"
  wait_for_dpkg
  apt-get update -qq
  apt-get install -y -qq build-essential pkg-config libssl-dev git curl clang lld python3
else
  echo "apt SKIPPED - every dependency already present"
fi

if ! command -v cargo >/dev/null 2>&1; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
fi
. "$HOME/.cargo/env"
# ⚠ see the header -- not optional. Idempotent, so unconditional is fine, but report it:
# a silent success here and a silent no-op look identical in the log.
if rustup target list --installed | grep -qx x86_64-unknown-linux-musl; then
  echo "musl target already installed"
else
  rustup target add x86_64-unknown-linux-musl && echo "musl target ADDED"
fi

[ -d ~/kayfabe ] || git clone -q https://github.com/reindertpelsma/kayfabe.git ~/kayfabe
cd ~/kayfabe
git fetch -q origin && git reset -q --hard origin/master
echo "HEAD=$(git rev-parse --short HEAD)"

# ⊘ Do NOT pipe cargo into tail: `cargo build | tail` makes $? the status of TAIL, which
#    always succeeds, so a FAILED build reports success. That exact bug produced a green
#    provisioning run over a workspace that had not compiled (2026-09-06). Capture the
#    status directly and read the log from a file.
set +e
cargo build --workspace > /tmp/kayfabe-build.log 2>&1
CARGO_RC=$?
set -e
echo "CARGO_RC=$CARGO_RC"
if [ "$CARGO_RC" -ne 0 ]; then
  echo "--- build errors ---"
  grep -E "^error|panicked at|could not compile" /tmp/kayfabe-build.log | head -20
fi
echo "PROVISION_DONE rc=$CARGO_RC $(date -Is)"
exit "$CARGO_RC"
