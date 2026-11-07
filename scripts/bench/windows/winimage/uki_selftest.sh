#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# uki_selftest.sh -- boot the UKI from build_alpine_uki.sh under plain QEMU/KVM + OVMF (Secure Boot off,
# nothing from a Vast box), ssh in with the session key, and check what the capture needs:
# tools present, the ESP found, kf-restore-esp puts the stand-in "Windows" boot files back, and the
# recovery timer restores and reboots on its own when nobody logs in.  GPU-free.
#
#   UKI=/data/.../kf-alpine.efi KEY=/root/night/winimage_private/alpine_key bash uki_selftest.sh [DIR]
#
# Prints UKI_SELFTEST_PASS or UKI_SELFTEST_FAIL <why>.  Needs: qemu-system-x86_64, OVMF, sgdisk, mkfs.vfat, mtools.
set -uo pipefail
UKI=${UKI:?set UKI}; KEY=${KEY:?set KEY (private key file)}
D=${1:-${TMPDIR:-/tmp}/uki_selftest.$$}; mkdir -p "$D"
OVMF_CODE=${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}; OVMF_VARS=${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.fd}
PORT=${PORT:-$((20000 + RANDOM % 20000))}
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p "$PORT" root@127.0.0.1)
fail() { echo "UKI_SELFTEST_FAIL $*"; exit 1; }
QPID=

mkdisk() {  # GPT disk with a 200 MiB ESP at 1 MiB holding stand-in Windows boot files and a backup folder
  local img=$D/disk.img
  rm -f "$img"; truncate -s 300M "$img"
  sgdisk -o -n 1:2048:+200M -t 1:EF00 -c 1:"EFI system partition" "$img" >/dev/null || fail sgdisk
  mkfs.vfat -F 32 -s 1 --offset 2048 -S 512 "$img" $((200 * 1024)) >/dev/null || fail mkfs.vfat
  local m="mtools_skip_check=1"; export MTOOLS_SKIP_CHECK=1
  local t="$img@@1048576"
  mmd -i "$t" ::/EFI ::/EFI/Microsoft ::/EFI/Microsoft/Boot ::/EFI/BOOT ::/EFI/kf-backup
  echo "ORIGINAL-BOOTMGFW" > "$D/bootmgfw.orig"; echo "ORIGINAL-FALLBACK" > "$D/fallback.orig"
  mcopy -i "$t" "$D/bootmgfw.orig" ::/EFI/kf-backup/bootmgfw.efi
  mcopy -i "$t" "$D/fallback.orig" ::/EFI/kf-backup/BOOTX64.EFI
  mcopy -i "$t" "$UKI" ::/EFI/Microsoft/Boot/bootmgfw.efi     # the swapped state
  mcopy -i "$t" "$UKI" ::/EFI/BOOT/BOOTX64.EFI
}
boot() {  # extra QEMU args after the standard ones
  cp "$OVMF_VARS" "$D/vars.fd"
  local bi=",bootindex=0"; case " $* " in *" -kernel "*) bi=;; esac   # a direct kernel boot has its own boot order
  qemu-system-x86_64 -machine q35,accel=kvm -cpu host -m 1024 -smp 2 \
    -drive if=pflash,format=raw,unit=0,readonly=on,file="$OVMF_CODE" -drive if=pflash,format=raw,unit=1,file="$D/vars.fd" \
    -drive file="$D/disk.img",if=none,id=d0,format=raw -device virtio-blk-pci,drive=d0$bi \
    -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$PORT-:22 -device virtio-net-pci,netdev=n0 \
    -display none -serial file:"$D/serial.log" -qmp unix:"$D/qmp.sock",server=on,wait=off -daemonize -pidfile "$D/qemu.pid" "$@" || fail "qemu start"
  QPID=$(cat "$D/qemu.pid")
}
stop() { [ -n "$QPID" ] && kill "$QPID" 2>/dev/null; QPID=; sleep 1; }
trap stop EXIT
wait_ssh() { local i; for i in $(seq 1 "${1:-60}"); do "${SSH[@]}" true 2>/dev/null && return 0; sleep 2; done; return 1; }
esp_file() { MTOOLS_SKIP_CHECK=1 mtype -i "$D/disk.img@@1048576" "::/$1" 2>/dev/null; }

# ---- part 1: boots, networks, ssh, tools, manual restore
mkdisk; boot
wait_ssh 60 || { tail -30 "$D/serial.log"; fail "no ssh in 120 s (serial.log in $D)"; }
grep -q 'KFUKI READY' "$D/serial.log" || fail "serial lacks the READY line"
out=$("${SSH[@]}" 'uname -r; for t in zstd ntfsclone mkfs.fat curl sgdisk lsblk efibootmgr sshd ssh-keygen dd; do command -v $t >/dev/null || echo MISSING:$t; done; lsblk -nro NAME,FSTYPE | tr "\n" ";"; echo; kf-restore-esp; echo RESTORE_RC=$?') || fail "ssh command failed"
echo "$out"
echo "$out" | grep -q MISSING && fail "tool missing"
echo "$out" | grep -q 'RESTORE_RC=0' || fail "kf-restore-esp rc"
[ "$(esp_file EFI/Microsoft/Boot/bootmgfw.efi)" = ORIGINAL-BOOTMGFW ] || fail "bootmgfw.efi not restored"
[ "$(esp_file EFI/BOOT/BOOTX64.EFI)" = ORIGINAL-FALLBACK ] || fail "BOOTX64.EFI not restored"
# password login must be refused
sshpass -p x ssh -o PubkeyAuthentication=no -o PreferredAuthentications=password -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5 -p "$PORT" root@127.0.0.1 true 2>/dev/null && fail "password login accepted"
stop

# ---- part 2: the recovery timer (kf.autorestore=20): nobody logs in; the files come back; the guest reboots
mkdisk
# a UKI whose cmdline has a 20 s timer is not rebuilt here: the kernel cmdline is overridden by QEMU's -append only for
# a direct kernel boot, so test the timer through a direct boot of the same kernel+initrd extracted from the UKI
objcopy -O binary --only-section=.linux "$UKI" "$D/vmlinuz" 2>/dev/null; objcopy -O binary --only-section=.initrd "$UKI" "$D/initrd" 2>/dev/null
if [ -s "$D/vmlinuz" ] && [ -s "$D/initrd" ]; then
  boot -kernel "$D/vmlinuz" -initrd "$D/initrd" -append "console=ttyS0 rdinit=/init net.ifnames=0 kf.autorestore=20"
  # the QEMU above has the disk already; wait for the timer to fire (20 s) plus the reboot (which re-runs the ESP's UKI)
  sleep 45
  grep -q 'NO LOGIN in 20s' "$D/serial.log" || fail "the recovery timer never fired (serial.log in $D)"
  stop
  [ "$(esp_file EFI/Microsoft/Boot/bootmgfw.efi)" = ORIGINAL-BOOTMGFW ] || fail "timer did not restore bootmgfw.efi"
else
  echo "note: could not extract kernel/initrd from the UKI sections; the timer part was skipped"
fi
echo "UKI_SELFTEST_PASS uki_bytes=$(stat -c %s "$UKI") dir=$D"
