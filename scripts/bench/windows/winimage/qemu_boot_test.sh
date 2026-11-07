#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# qemu_boot_test.sh -- boot a captured Windows disk image under plain QEMU/KVM in the CI hardware shape of
# scripts/bench/windows/win_vm.sh (q35 + SMM, OVMF pflash with per-VM vars, swtpm tpm-crb, virtio-blk, virtio-net,
# -vga std, fixed -uuid/-smbios, -rtc base=utc, user networking with an ssh hostfwd), on a COPY-ON-WRITE overlay of the
# image (the image is never written), and record what happens: QMP screendumps (PNG) every EVERY seconds, whether the
# guest's ssh answers, and a one-line verdict. No kayfabe device: this is the "does the disk boot in our shape" test.
#
#   qemu_boot_test.sh IMAGE.qcow2 WORKDIR [options]
#     --sb ms|off      ms (default): OVMF_CODE_4M.secboot + OVMF_VARS_4M.ms (Microsoft keys enrolled, Secure Boot ON)
#                      off: OVMF_CODE_4M + OVMF_VARS_4M (no keys, Secure Boot OFF)
#     --ram MB (8192)  --smp N (4)  --wait SECONDS (900)  --every SECONDS (20)  --port N (ssh hostfwd, default random)
#     --key FILE       ssh private key to try for user `vast` (the key windows-on-vast authorized), default ~/.ssh/id_ed25519
#     --keep           leave the VM running at the end (default: guest shutdown via ssh if it answers, else QMP quit)
#     --nic virtio|e1000   NIC model (default virtio-net-pci)     --disk virtio|sata|nvme (default virtio-blk)
#
# Output: WORKDIR/shots/tNNNN.png, WORKDIR/serial.log, WORKDIR/boot_test.log and the last line
#   BOOT_TEST_RESULT sb=... disk=... nic=... first_change_s=N ssh_up_s=N|never last_screen=<sha> verdict=<ssh_up|booted_no_ssh|no_boot_progress>
# A person (or the caller) reads the last screenshots to classify a non-ssh outcome (INACCESSIBLE_BOOT_DEVICE, recovery,
# reactivation, ...); this script never claims more than "ssh answered" or "the screen stopped changing at X".
set -uo pipefail
IMG=${1:?image.qcow2}; W=${2:?workdir}; shift 2
SB=ms RAM=8192 SMP=4 WAIT=900 EVERY=20 PORT=$((20000 + RANDOM % 20000)) KEY=$HOME/.ssh/id_ed25519 KEEP=0 NIC=virtio DISKBUS=virtio
while [ $# -gt 0 ]; do case $1 in
  --sb) SB=$2; shift 2;; --ram) RAM=$2; shift 2;; --smp) SMP=$2; shift 2;; --wait) WAIT=$2; shift 2;; --every) EVERY=$2; shift 2;;
  --port) PORT=$2; shift 2;; --key) KEY=$2; shift 2;; --keep) KEEP=1; shift;; --nic) NIC=$2; shift 2;; --disk) DISKBUS=$2; shift 2;;
  *) echo "unknown option $1" >&2; exit 2;; esac; done
HERE=$(cd "$(dirname "$0")" && pwd); QMPPY=$HERE/../qmp.py
case $SB in
  ms)  CODE=${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.secboot.fd}; VARS_T=${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.ms.fd};;
  off) CODE=${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd};         VARS_T=${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.fd};;
  *) echo "--sb must be ms or off" >&2; exit 2;;
esac
for f in "$IMG" "$CODE" "$VARS_T" "$QMPPY"; do [ -e "$f" ] || { echo "missing $f" >&2; exit 2; }; done
for t in qemu-system-x86_64 qemu-img swtpm swtpm_setup pnmtopng; do command -v $t >/dev/null || { echo "missing tool $t" >&2; exit 2; }; done
rm -rf "$W"; mkdir -p "$W/shots" "$W/tpm" "$W/run"; chmod 700 "$W/tpm"
LOG=$W/boot_test.log
say() { echo "$(date +%T) $*" | tee -a "$LOG"; }
say "BOOT_TEST_START image=$IMG sb=$SB ram=$RAM smp=$SMP port=$PORT nic=$NIC disk=$DISKBUS pid=$$"

qemu-img create -q -f qcow2 -b "$(readlink -f "$IMG")" -F qcow2 "$W/overlay.qcow2" || exit 1
cp "$VARS_T" "$W/OVMF_VARS.fd"
swtpm_setup --tpm2 --tpmstate "$W/tpm" --create-ek-cert --create-platform-cert --lock-nvram --pcr-banks sha256 --logfile "$W/swtpm_setup.log" >/dev/null 2>&1 || { say "swtpm_setup failed"; exit 1; }
swtpm socket --tpm2 --tpmstate "dir=$W/tpm,mode=0600" --ctrl "type=unixio,path=$W/run/swtpm.sock,mode=0600" --pid "file=$W/run/swtpm.pid" --terminate --daemon --log "file=$W/swtpm.log,level=20"
for _ in $(seq 1 50); do [ -S "$W/run/swtpm.sock" ] && break; sleep 0.2; done

case $NIC in virtio) NICDEV=virtio-net-pci;; e1000) NICDEV=e1000e;; *) echo "bad --nic" >&2; exit 2;; esac
case $DISKBUS in
  virtio) DISKDEV=(-device virtio-blk-pci,drive=sys,bootindex=0);;
  sata)   DISKDEV=(-device ich9-ahci,id=ahci0 -device ide-hd,drive=sys,bus=ahci0.0,bootindex=0);;
  nvme)   DISKDEV=(-device nvme,drive=sys,serial=kfboot0,bootindex=0);;
  *) echo "bad --disk" >&2; exit 2;;
esac
QEMU=(qemu-system-x86_64 -name kf-bootest,debug-threads=on
  -machine pc-q35-10.2,accel=kvm,smm=on -m "$RAM" -smp "$SMP,sockets=1,cores=$SMP,threads=1"
  -cpu host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash
  -global driver=cfi.pflash01,property=secure,value=on
  -drive "if=pflash,format=raw,unit=0,readonly=on,file=$CODE" -drive "if=pflash,format=raw,unit=1,file=$W/OVMF_VARS.fd"
  -global ICH9-LPC.disable_s3=1 -global ICH9-LPC.disable_s4=1
  -rtc base=utc,driftfix=slew -global kvm-pit.lost_tick_policy=discard
  -chardev "socket,id=chrtpm,path=$W/run/swtpm.sock" -tpmdev emulator,id=tpm0,chardev=chrtpm -device tpm-crb,tpmdev=tpm0
  -drive "file=$W/overlay.qcow2,if=none,id=sys,format=qcow2,discard=unmap" "${DISKDEV[@]}"
  -netdev "user,id=n0,restrict=off,hostfwd=tcp:127.0.0.1:$PORT-:22" -device "$NICDEV,netdev=n0,mac=52:54:00:4b:46:58"
  -uuid 5d9d8f0e-8b5f-4c1d-9a39-6a7b1c2d3e4f -smbios type=1,serial=KFBOOTEST01
  -object rng-random,id=rng0,filename=/dev/urandom -device virtio-rng-pci,rng=rng0
  -vga std -display none -serial "file:$W/serial.log" -qmp "unix:$W/run/qmp.sock,server=on,wait=off"
  -msg timestamp=on -daemonize -pidfile "$W/run/qemu.pid" -action panic=none)
printf '%q ' "${QEMU[@]}" > "$W/qemu_cmdline.txt"
"${QEMU[@]}" 2> "$W/qemu.err" || { say "qemu failed to start: $(head -3 "$W/qemu.err")"; exit 1; }
QPID=$(cat "$W/run/qemu.pid"); say "QEMU_STARTED pid=$QPID"
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p "$PORT" vast@127.0.0.1)

t0=$(date +%s); last=; first_change=; ssh_up=; n=0; stable=0
while :; do
  el=$(( $(date +%s) - t0 ))
  [ -d "/proc/$QPID" ] || { say "QEMU_EXITED at ${el}s"; break; }
  f=$(printf '%s/shots/t%04d' "$W" "$el")
  if python3 "$QMPPY" "$W/run/qmp.sock" cmd screendump "{\"filename\":\"$f.ppm\"}" >/dev/null 2>&1 && pnmtopng "$f.ppm" > "$f.png" 2>/dev/null; then
    h=$(sha256sum < "$f.ppm" | cut -c1-12); rm -f "$f.ppm"
    if [ "$h" != "$last" ]; then [ -z "$first_change" ] && [ -n "$last" ] && first_change=$el; stable=0; else stable=$((stable + 1)); rm -f "$f.png"; fi
    last=$h
  fi
  # ssh answers: the guest booted, has a NIC driver, and authorized the same key windows-on-vast installed
  if [ -z "$ssh_up" ] && "${SSH[@]}" 'echo SSH_UP' 2>/dev/null | grep -q SSH_UP; then ssh_up=$el; say "SSH_UP at ${el}s"; fi
  [ -n "$ssh_up" ] && [ $((el - ssh_up)) -ge 30 ] && break
  [ "$el" -ge "$WAIT" ] && { say "WAIT_ELAPSED ${WAIT}s"; break; }
  [ "$stable" -ge 15 ] && [ -z "$ssh_up" ] && [ "$el" -ge 240 ] && { say "SCREEN_STATIC for $((stable * EVERY))s with no ssh: stopping early"; break; }
  sleep "$EVERY"
done
# the final frame, always kept
f=$W/shots/final
python3 "$QMPPY" "$W/run/qmp.sock" cmd screendump "{\"filename\":\"$f.ppm\"}" >/dev/null 2>&1 && pnmtopng "$f.ppm" > "$f.png" 2>/dev/null && rm -f "$f.ppm"
if [ "$KEEP" = 0 ] && [ -d "/proc/$QPID" ]; then
  [ -n "$ssh_up" ] && "${SSH[@]}" 'shutdown /s /t 0' >/dev/null 2>&1
  for _ in $(seq 1 30); do [ -d "/proc/$QPID" ] || break; sleep 2; done
  [ -d "/proc/$QPID" ] && python3 "$QMPPY" "$W/run/qmp.sock" cmd quit >/dev/null 2>&1
fi
verdict=no_boot_progress; [ -n "$first_change" ] && verdict=booted_no_ssh; [ -n "$ssh_up" ] && verdict=ssh_up
say "BOOT_TEST_RESULT sb=$SB disk=$DISKBUS nic=$NIC first_change_s=${first_change:-never} ssh_up_s=${ssh_up:-never} screens=$(ls "$W/shots" | wc -l) last_screen=${last:-none} verdict=$verdict"
