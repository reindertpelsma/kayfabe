#!/usr/bin/env bash
# Trusted host. Stop the owner's Windows 11 VM (run 2990), write a local-GPO startup script + the QEMU guest agent MSI
# onto its NTFS volume (file-only: no registry edit), so the next boot installs the agent as SYSTEM and creates an
# admin account for the harness with auto-logon. The password goes to a 0600 host file, never printed.
set -uo pipefail
W=/var/lib/kf-windows-20261005; D=$W/win11-full-b; RUN=$W/boundary-kayfabe-2990
echo "== stopping run 2990"
python3 - <<PY
import socket, json
try:
    s = socket.socket(socket.AF_UNIX); s.connect("$RUN/qmp.sock"); f = s.makefile("rw"); f.readline()
    f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n"); f.flush(); f.readline()
    f.write(json.dumps({"execute": "quit"}) + "\n"); f.flush()
except Exception as e:
    print("qmp:", e)
PY
for i in $(seq 1 20); do pgrep -f "[b]oundary-kayfabe-2990" > /dev/null || break; sleep 2; done
touch /tmp/kf-stop-claude-manual; sleep 8; rm -f /tmp/kf-stop-claude-manual
pgrep -f "[b]oundary-kayfabe-2990" > /dev/null && { echo "VM still running, abort"; exit 3; }
echo "== password"
PWF=$D/kfadmin.password
rm -f $PWF
[ -f $PWF ] || { python3 -c "import secrets,string; print(''.join(secrets.choice(string.ascii_letters+string.digits) for _ in range(10))+'Aa1!')" > $PWF; chmod 600 $PWF; }
PW=$(cat $PWF)
echo "== mounting NTFS read-write"
modprobe nbd max_part=8 2>/dev/null
qemu-nbd -c /dev/nbd1 $D/windows11.qcow2 || exit 4
sleep 2; partprobe /dev/nbd1; sleep 1
M=/mnt/kfinject; mkdir -p $M
ok=0
for p in 3 4 2; do
  if mount -t ntfs-3g -o rw,force,remove_hiberfile /dev/nbd1p$p $M 2> /tmp/claude-mount.err && [ -d $M/Windows ]; then ok=1; echo "mounted p$p rw"; break; fi
  umount $M 2> /dev/null
done
[ $ok = 1 ] || { cat /tmp/claude-mount.err; qemu-nbd -d /dev/nbd1; exit 5; }
mkdir -p $M/kf
# the agent MSI from the virtio ISO
mkdir -p /mnt/vio2; mount -o loop,ro /var/lib/vast-windows-test-media/virtio.iso /mnt/vio2
cp /mnt/vio2/guest-agent/qemu-ga-x86_64.msi $M/kf/qga.msi
cp /tmp/kf-agent.ps1 $M/kf/agent.ps1; mkdir -p $M/kf/vioserial; cp /mnt/vio2/vioserial/w11/amd64/* $M/kf/vioserial/
umount /mnt/vio2
# local GPO startup script (files only)
G=$M/Windows/System32/GroupPolicy
mkdir -p $G/Machine/Scripts/Startup $G/User
printf '[General]\r\ngPCMachineExtensionNames=[{42B5FAAE-6536-11D2-AE5A-0000F87571E3}{40B6664F-4972-11D1-A7CA-0000F87571E3}]\r\nVersion=1\r\n' > $G/gpt.ini
printf '\r\n[Startup]\r\n0CmdLine=kfsetup.cmd\r\n0Parameters=\r\n' > $G/Machine/Scripts/scripts.ini
cat > $G/Machine/Scripts/Startup/kfsetup.cmd <<EOF
@echo off
rem kayfabe harness setup, runs as SYSTEM at boot. Idempotent.
if not exist C:\\kf mkdir C:\\kf
echo %date% %time% start >> C:\\kf\\setup.log
powercfg /h off >> C:\\kf\\setup.log 2>&1
pnputil /add-driver C:\\kf\\vioserial\\vioser.inf /install >> C:\\kf\\setup.log 2>&1
schtasks /create /tn kfagent /sc onstart /ru SYSTEM /rl highest /tr "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\agent.ps1" /f >> C:\\kf\\setup.log 2>&1
schtasks /run /tn kfagent >> C:\\kf\\setup.log 2>&1
echo Y| net user kfadmin $PW /add >> C:\\kf\\setup.log 2>&1
net localgroup Administrators kfadmin /add >> C:\\kf\\setup.log 2>&1
reg add "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon" /v AutoAdminLogon /t REG_SZ /d 1 /f >> C:\\kf\\setup.log 2>&1
reg add "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon" /v DefaultUserName /t REG_SZ /d kfadmin /f >> C:\\kf\\setup.log 2>&1
reg add "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon" /v DefaultPassword /t REG_SZ /d $PW /f >> C:\\kf\\setup.log 2>&1
reg add "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon" /v DefaultDomainName /t REG_SZ /d . /f >> C:\\kf\\setup.log 2>&1
echo %date% %time% done >> C:\\kf\\setup.log
EOF
sed -i 's/$/\r/' $G/Machine/Scripts/Startup/kfsetup.cmd
ls -la $G $G/Machine/Scripts $G/Machine/Scripts/Startup $M/kf | cut -c1-90
sync; umount $M; qemu-nbd -d /dev/nbd1
echo INJECT_DONE
