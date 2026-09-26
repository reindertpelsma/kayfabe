#!/usr/bin/env bash
# ★ STAGE ONE FAT-GUEST IMAGE PER GUEST DRIVER VERSION — a qcow2 overlay on the bench image
# with that version's driver installed the way an operator would (the .run, open modules),
# for the CUDA ladder (`cuda_ladder.sh guest`, `KF_GUEST_IMG=`).
#
#   usage: stage_fat_guest.sh <version> [<stage-root>]
#   needs: <stage-root>/<version>/NVIDIA-Linux-x86_64-<version>.run (stage_guest_driver.sh keeps it)
#   result: /workspace/bench/guest-<version>.qcow2 (+ .STAGED beside it, written last)
#
# ## Why an overlay, and why the .run
#
# The bench image (`guest.qcow2`, built by `provision_bench_tree.sh`) carries the host's version.
# An overlay keeps it untouched — every version's image is a small delta, and the base stays the
# known-good 580.159.04 guest. ⊘ The base must never be written while an overlay exists.
# The .run with `-m=kernel-open` is the same install path `provision_bench_tree.sh` uses, so the
# only variable between two images is the version.
#
# ⚠ Verified on CONTENT, not the installer's exit code (the w474 lesson in that script): the
# guest's `modinfo -F version nvidia` AND the version of the libcuda it will load must both
# equal <version>, or the stage fails and no STAGED marker is written.
set -uo pipefail
V=${1:?usage: stage_fat_guest.sh <version> [<stage-root>]}
ROOT=${2:-/workspace/drivers}
BENCH=${BENCH_DIR:-/workspace/bench}
BASE=$BENCH/guest.qcow2
IMG=$BENCH/guest-$V.qcow2
RUN=$ROOT/$V/NVIDIA-Linux-x86_64-$V.run
PORT=${KF_STAGE_SSH_PORT:-2229}
say() { echo "[$(date -Is)] fat $V: $*"; }
die() { say "⊘ $*"; exit 1; }

[ -s "$RUN" ] || die "no $RUN — run stage_guest_driver.sh $V first (or place the .run there)"
[ -f "$BASE" ] || die "no base image $BASE"
[ -f "$BENCH/guest_key" ] || die "no $BENCH/guest_key"
command -v qemu-system-x86_64 >/dev/null || die "no stock qemu-system-x86_64 (apt qemu-system-x86)"
rm -f "$IMG.STAGED"
if [ ! -f "$IMG" ]; then
    qemu-img create -q -f qcow2 -b "$BASE" -F qcow2 "$IMG" || die "overlay create failed"
fi

PIDF=$BENCH/stage-$V.pid
qemu-system-x86_64 -enable-kvm -cpu host -m 8G -smp "${KF_STAGE_SMP:-4}" -display none \
    -drive if=virtio,file="$IMG",format=qcow2 \
    -netdev user,id=n0,hostfwd=tcp::$PORT-:22 -device virtio-net-pci,netdev=n0 \
    -serial file:"$BENCH/stage-$V-serial.log" -daemonize -pidfile "$PIDF" || die "qemu did not start"
trap 'kill "$(cat "$PIDF" 2>/dev/null)" 2>/dev/null' EXIT
SSHO=(-i "$BENCH/guest_key" -p "$PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
      -o LogLevel=ERROR -o ConnectTimeout=8 -o ServerAliveInterval=15 -o ServerAliveCountMax=3)
GS() { timeout 120 ssh -n "${SSHO[@]}" ubuntu@127.0.0.1 "$@"; }
GSL() { timeout 1800 ssh -n "${SSHO[@]}" ubuntu@127.0.0.1 "$@"; }
for i in $(seq 1 40); do GS true >/dev/null 2>&1 && break; sleep 5; done
GS true >/dev/null 2>&1 || die "guest never answered ssh (serial: $BENCH/stage-$V-serial.log)"
say "guest up; installed now: $(GS 'modinfo -F version nvidia 2>/dev/null' | tr -d '\r')"

# /var/tmp is on disk (the guest's /tmp is a tmpfs).
scp -q -P "$PORT" -i "$BENCH/guest_key" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR "$RUN" ubuntu@127.0.0.1:/var/tmp/nv-$V.run || die "scp of the .run failed"
say "uninstall the previous .run install, install $V (open modules)"
GSL "sudo /usr/bin/nvidia-uninstall --silent >/dev/null 2>&1; \
     sudo sh /var/tmp/nv-$V.run --silent --no-x-check --no-nouveau-check --no-questions -m=kernel-open -j$(nproc); \
     echo NVRUN_RC=\$?" 2>&1 | tail -4

MOD=$(GS 'modinfo -F version nvidia 2>/dev/null' | tr -d '\r')
CUDA=$(GS 'ls /usr/lib/x86_64-linux-gnu/libcuda.so.* 2>/dev/null | grep -oE "[0-9]+\.[0-9]+(\.[0-9]+)?$" | sort -V | tail -1' | tr -d '\r')
say "guest modinfo=$MOD libcuda=$CUDA"
[ "$MOD" = "$V" ] || die "the guest's nvidia.ko is '$MOD', not $V — see /var/log/nvidia-installer.log in the guest"
[ "$CUDA" = "$V" ] || die "the guest's libcuda is '$CUDA', not $V"
GS "sudo rm -f /var/tmp/nv-$V.run; sudo sync" >/dev/null 2>&1
GS "sudo poweroff" >/dev/null 2>&1
for i in $(seq 1 30); do kill -0 "$(cat "$PIDF" 2>/dev/null)" 2>/dev/null || break; sleep 2; done
trap - EXIT
kill -0 "$(cat "$PIDF" 2>/dev/null)" 2>/dev/null && { kill "$(cat "$PIDF")"; die "guest did not power off"; }
{ echo "version=$V"; echo "image=$IMG"; echo "base=$BASE"; echo "staged=$(date -Is)"; } > "$IMG.STAGED"
say "STAGED ✔ $IMG"
