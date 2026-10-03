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
die() { echo "provision_box: $*" >&2; exit 1; }
echo "PROVISION_START $(date -Is)"

export DEBIAN_FRONTEND=noninteractive

# ★★★★★ **PICK A MIRROR THAT CAN ACTUALLY SERVE US, AND MEASURE RATHER THAN ASSUME.**
#
# `[measured w439, vast 50585481]` the box's default `archive.ubuntu.com` served
# **13 344 B/s** while GitHub on the SAME box served **1 434 152 B/s** — a 107x gap, and not a
# general egress problem. `apt-get install build-essential …` ran for **44 minutes** and had
# installed nothing; `/opt/qemu-src`, `~/.cargo` and `/workspace/bench` did not exist.
#
# ⊘ The host preflight passed that box: it checks that https egress WORKS, not how fast. A
# reachability check cannot distinguish a usable mirror from one that will never finish.
#
# ⚠ Do not hardcode a favourite. Mirror speed is a property of where the box IS, and this
# session measured `azure.archive.ubuntu.com` at 2.5 MB/s and `mirror.enzu.com` at 295 kB/s
# from one machine. Race a few and take the winner; keep the default in the list so a box
# where it is genuinely fastest is unaffected.
pick_apt_mirror() {
  local best="" best_speed=0 m speed
  for m in archive.ubuntu.com/ubuntu azure.archive.ubuntu.com/ubuntu \
           mirrors.edge.kernel.org/ubuntu; do
    speed=$(timeout 12 curl -s -o /dev/null -w "%{speed_download}" \
            "http://$m/dists/jammy/Release" 2>/dev/null || echo 0)
    speed=${speed%%.*}
    echo "   mirror $m -> ${speed:-0} B/s"
    if [ "${speed:-0}" -gt "$best_speed" ]; then best_speed=$speed; best=$m; fi
  done
  # ⊘ A floor, not a preference: below this, provisioning does not finish in any timeout we
  # would sanely set, and the honest move is to say so rather than run for an hour.
  if [ "$best_speed" -lt 200000 ]; then
    echo "⊘⊘ NO USABLE APT MIRROR — fastest was $best at ${best_speed} B/s (< 200 kB/s)."
    echo "   Provisioning this box will not finish. Destroy it and rent another."
    return 1
  fi
  if [ "$best" != "archive.ubuntu.com/ubuntu" ]; then
    echo "== switching apt to $best (${best_speed} B/s)"
    sed -i "s|http://archive.ubuntu.com/ubuntu|http://$best|g" /etc/apt/sources.list
    sed -i "s|http://archive.ubuntu.com/ubuntu|http://$best|g" \
        /etc/apt/sources.list.d/*.list 2>/dev/null || true
  fi
}
pick_apt_mirror || exit 1

# ⚠ A FRESH CLOUD BOX RUNS `unattended-upgrades` AT BOOT and it holds the dpkg lock.
# Measured on vast instance 50013922, 2026-09-06: provisioning launched ~7 minutes after
# first boot and died instantly with
#   E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 7603
# `apt-get` exits 100 without installing anything. ⇒ The failure is a RACE WITH THE BOX,
# not with our code, and retrying by hand a minute later "fixes" it — which is exactly why
# it never gets written down and bites the next person instead. Wait for the lock.
# ⊘ [2026-09-27] Moved up from below the in-flight check, which now waits too (see there).
# ⊘ [2026-09-27] The lock test was `fuser` alone, and `fuser` is psmisc, which nothing in the
# provisioning chain installed (it is in the list below now). Without it `fuser` exits 127, the
# loop reads that as "nobody holds the lock", and the wait is a silent no-op ("free after 0s").
# ⇒ Until psmisc is in, ask the kernel's lock table: `lslocks` (util-linux, which is Essential, so
# it is on every Ubuntu image) lists the fcntl locks that apt and dpkg hold, by path. Process names
# are the last resort, for an image without lslocks.
# ⚠ Never the process name `unattended-upgr`: unattended-upgrades.service keeps a helper up from
# boot to shutdown (`unattended-upgrade-shutdown --wait-for-signal`, a python3 script, so its comm
# is `unattended-upgr` too) that never takes the lock. Matching it made every wait run to its bound
# on a box without psmisc (1200 s, then 600 s and exit 1: the run this fallback was added for).
# [reproduced 2026-09-27 with a python3 shebang script of that name: `pgrep -x unattended-upgr`
# rc=0 with no upgrade running.] The upgrade itself is matched on its command line, which the kernel
# builds from the shebang as `/usr/bin/python3 /usr/bin/unattended-upgrade [args]`.
uu_running() {
  pgrep -f '^[^ ]*python3[^ ]* [^ ]*/unattended-upgrade( |$)' >/dev/null 2>&1
}
dpkg_busy() {
  local held
  if command -v fuser >/dev/null 2>&1; then
    fuser /var/lib/dpkg/lock-frontend /var/lib/apt/lists/lock >/dev/null 2>&1
  elif held=$(lslocks -n -o PATH 2>/dev/null); then
    grep -qE '^/var/lib/(dpkg/lock-frontend|apt/lists/lock)' <<<"$held"
  else
    uu_running || pgrep -x 'apt|apt-get|dpkg' >/dev/null 2>&1
  fi
}
wait_for_dpkg() {  # [MAX_SECONDS], default 600
  local waited=0 max=${1:-600}
  while dpkg_busy; do
    [ "$waited" -ge "$max" ] && { echo "DPKG_LOCK_TIMEOUT after ${waited}s"; return 1; }
    [ $((waited % 60)) -eq 0 ] && echo "waiting for dpkg lock (${waited}s)"
    sleep 10; waited=$((waited + 10))
  done
  echo "dpkg lock free after ${waited}s"
}

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
# ⊘⊘ [2026-09-27] THIS CHECK KILLED THE RUN IT WARNS ABOUT. Under `set -euo pipefail` its report,
# `grep … | sort -u | head`, fails whenever the run in flight names no kernel package (grep exit
# 1) or has not written its log yet (exit 2: e.g. the boot-time `apt-get update` holds the lock).
# pipefail made that the pipeline's status and `set -e` ended provisioning HERE, before the clone
# — provision_full.sh then prints BOX_RC=1 and NO_REPO. [reproduced 2026-09-27, this block
# isolated: rc=2 with no log, rc=1 with a kernel-free log; it survived only when a kernel
# package WAS listed.] ⇒ The report is never fatal, and the run in flight is WAITED OUT, bounded
# at 1200 s (provision_host_driver.sh's bound for the same lock, which it needs next anyway):
# whether it installed a kernel can only be answered once it has finished, so the answer is
# computed below instead of being left to whoever reads this log.
# ⊘ The entry test was `pgrep -x unattended-upgr`, which also matches the always-running shutdown
# helper (see the ⚠ at dpkg_busy), so every box with unattended-upgrades reported a run in flight.
if uu_running || dpkg_busy; then
  echo "⚠ an unattended upgrade is ALREADY IN FLIGHT -- kernel packages its log names so far:"
  { grep -oE "linux-(image|headers|generic)[a-z0-9.-]*" /var/log/unattended-upgrades/unattended-upgrades.log 2>/dev/null | sort -u | head; } || true
  wait_for_dpkg 1200 || echo "⚠ still locked after 1200s -- continuing; the apt step below waits again and stops loudly"
fi
# ★ [2026-09-27] The kernel question, answered on content rather than from the log: a run that
# FINISHED before this script started (never seen in flight) installs a kernel just the same.
newest_kernel=$(find /boot -maxdepth 1 -name 'vmlinuz-*' -printf '%f\n' 2>/dev/null \
                  | sed 's/^vmlinuz-//' | sort -V | tail -1 || true)
if [ -n "$newest_kernel" ] && [ "$newest_kernel" != "$(uname -r)" ]; then
  echo "⚠⚠ REBOOT_NEEDED running=$(uname -r) newest_installed=$newest_kernel -- REBOOT before installing the NVIDIA driver (see the header)"
fi

# ★ ASK BEFORE WAITING. ⊘ [2026-09-27] Partly superseded: a run found IN FLIGHT is now waited
# out above (bounded), because the kernel question needs it finished and the driver step needs
# the same lock next, so that wait moves earlier rather than being added (what is lost is only
# its overlap with the cargo build below). This probe still decides whether apt runs at all.
# Measured on the same box: `unattended-upgrade` held the lock for
# over nine minutes, while EVERY package below was already installed -- the image ships
# them. Waiting for a lock to run an install that would be a no-op is pure dead time, and
# on a 600s ceiling it can fail the run outright. So probe first and only touch apt if
# something is genuinely missing.
# ⊘⊘ `modprobe` and `lspci` are in this list because of a MEASURED failure, not caution.
# `[measured 2026-09-13, vastai/kvm:ubuntu_cli_22.04-2025-11-21]` that image ships WITHOUT
# `kmod` and `pciutils`. `provision_box.sh` passed happily — it needs neither — and then
# `provision_host_driver.sh` downloaded 397 MB, ran the NVIDIA installer, and died on:
#
#     ERROR: Unable to find the module utility `modprobe`
#
# ⚠ The installer's own exit status is IGNORED_ON_PURPOSE by that script, so the only thing
# that caught it was its `OPEN_MODULE=no` check at the end. The box looked provisioned and was
# not. ⇒ A dependency of a LATER step belongs in the FIRST step's list, because the later step
# is the one that cannot tell a missing tool from a broken swap.
# ★ [2026-09-27] Same rule, for the tools the later steps assume and nothing installed:
#   - `busybox`, `cpio`: build_fast_guest.sh dies without them (its `command -v` checks);
#   - `zstd`: the same builder decompresses the noble guest's `.ko.zst` modules with no check —
#     without it the initrd's loadorder names modules the initrd does not contain;
#   - `fuser` (psmisc): every wait_for_dpkg in the chain (here, provision_host_driver.sh,
#     provision_bench_tree.sh) — see the ⊘ at dpkg_busy.
# ⚠ busybox must be the STATIC one: the initrd carries shared libraries for the raw client only
# (build_fast_guest.sh, "carry what it needs"), and Ubuntu's `busybox` is dynamic (Depends:
# libc6; it also needs libresolv, which the client does not). A present-but-dynamic busybox
# therefore counts as missing, and `busybox-static` (Conflicts/Replaces: busybox) goes in.
# [checked 2026-09-27 on the extracted packages: `ldd` exits 1 on busybox-static's binary, 0 on
# busybox's.]
NEED=""
for b in gcc pkg-config git curl clang lld python3 modprobe lspci cpio zstd fuser; do
  command -v "$b" >/dev/null 2>&1 || NEED="$NEED $b"
done
[ -e /usr/include/openssl/ssl.h ] || NEED="$NEED libssl-dev"
# ldd exits non-zero for a static binary ("not a dynamic executable")
if ! command -v busybox >/dev/null 2>&1 || ldd "$(command -v busybox)" >/dev/null 2>&1; then
  NEED="$NEED busybox-static"
fi
if [ -n "$NEED" ]; then
  echo "apt needed for:$NEED"
  wait_for_dpkg
  apt-get update -qq
  apt-get install -y -qq build-essential pkg-config libssl-dev git curl clang lld python3 \
      kmod pciutils cpio zstd psmisc busybox-static
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

# ⊘⊘⊘ **THE BRANCH, EXPLICITLY, AND THE REVISION, PRINTED — w825.**
#
# This was `git clone -q <url> ~/kayfabe` with **no branch**, so it took the repo's DEFAULT
# branch. `[measured w825]` a freshly provisioned box therefore arrived at **`e24bc063`
# (w720l)** — the default branch, ~100 commits behind the work branch — and the first build
# against it failed on a cargo feature HEAD has. ⚠ The box looked correctly provisioned:
# `PROVISION_DONE rc=0`, driver swapped, cargo present, a repo at the expected path.
#
# ★ Same class as `REPO=${KAYFABE_REPO:-/root/kayfabe}` (w824) and as CLAUDE.md's oldest trap
# ("the bench silently served a binary built from `862c7c2` for weeks"): **an invisible default
# deciding which code a measurement is about.** ⇒ Name the branch, and PRINT what you got.
# ★★★ V3_SEC_P0 — the unprivileged QEMU user. The security model is an UNPRIVILEGED host process
# driving the GPU; kf3 refuses to realize while the process holds CAP_SYS_ADMIN
# (crates/kf-qemu/src/device.rs), so the bench must launch QEMU as a non-root user. The NVIDIA
# device nodes default to 0666 (NVreg_DeviceFileMode), so this user needs only the `kvm` group for
# /dev/kvm and a raised memlock (the launcher raises it as root before the setpriv drop). A system
# user with no login and no home.
KF_QEMU_USER=${KF_QEMU_USER:-kfqemu}
if ! id "$KF_QEMU_USER" >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin "$KF_QEMU_USER" \
        || die "could not create the unprivileged QEMU user $KF_QEMU_USER"
fi
getent group kvm >/dev/null 2>&1 || groupadd --system kvm
usermod -aG kvm "$KF_QEMU_USER" || die "could not add $KF_QEMU_USER to the kvm group"
# Belt-and-suspenders for any PAM-login path (the setpriv drop already carries root's raised
# rlimit across the uid change, which is the load-bearing one).
cat > /etc/security/limits.d/99-kfqemu.conf <<LIM
$KF_QEMU_USER - memlock unlimited
LIM
# The NVIDIA nodes are 0666 by default; assert it, and fix it if a box ever ships them tighter,
# so the dropped user can open /dev/nvidia*.
for n in /dev/nvidiactl /dev/nvidia0 /dev/nvidia-uvm; do
    [ -e "$n" ] && chmod o+rw "$n" 2>/dev/null || true
done
command -v setpriv >/dev/null 2>&1 || die "setpriv(1) missing (util-linux) — the QEMU privilege drop needs it"
echo "SEC_P0_QEMU_USER=$KF_QEMU_USER groups=$(id -nG "$KF_QEMU_USER" 2>/dev/null)"

KF_BRANCH=${KAYFABE_BRANCH:-w749-fable-legb}
if [ ! -d ~/kayfabe/.git ]; then
    git clone -q --branch "$KF_BRANCH" https://github.com/reindertpelsma/kayfabe.git ~/kayfabe \
        || die "clone of branch $KF_BRANCH failed"
else
    git -C ~/kayfabe fetch -q origin "$KF_BRANCH" \
        && git -C ~/kayfabe checkout -q -B "$KF_BRANCH" "origin/$KF_BRANCH" \
        || die "could not put ~/kayfabe on $KF_BRANCH"
fi
# ⊘ The revision is part of every claim this box will produce. Print it where the provisioning
# log keeps it, next to the artefacts.
echo "PROVISION_REPO=$HOME/kayfabe branch=$KF_BRANCH rev=$(git -C ~/kayfabe rev-parse --short HEAD)"
cd ~/kayfabe
# ⊘⊘⊘ w826: a `git reset --hard origin/master` sat HERE, AFTER the branch checkout above, and
# silently rewound the named branch to master — a box provisioned with KAYFABE_BRANCH=v3 built
# `e24bc063` (master). Only this HEAD line exposed it. The branch is set ONCE, above.
echo "HEAD=$(git rev-parse --short HEAD) branch=$(git branch --show-current)"

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
