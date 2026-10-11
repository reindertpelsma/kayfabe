#!/usr/bin/env bash
# session_prep.sh -- host side of the 2026-10-11 VFIO GL reference. Usage: session_prep.sh (run under `flock -o /tmp/kayfabe-fastguest.lock`)
# detach 4070, boot a fresh overlay of the baseline with the observer, set the vast password, sign in with keys, attach the exFAT app disk.
set -u
W=/var/lib/kf-windows-20261005; D=$W/vfio-gl-20261011; T=$D/tools; N=${BOOT:-1}; R=$D/boot$N; PW=kfsign7   # BOOT=2 REUSE=1: another boot on the same disk (4070 stays bound to vfio-pci)
export VDR_DIR=$D VDR_TRACE_BAR0=0
Q(){ timeout 15 python3 $W/boundary-tools/qmp.py $R/qmp.sock "$@"; }
G(){ timeout ${GT:-60} python3 $W/boundary-tools/qmp.py $R/qga.sock "$@"; }
key(){ Q cmd send-key "{\"keys\":[{\"type\":\"qcode\",\"data\":\"$1\"}]}" >/dev/null 2>&1; sleep 0.25; }
L(){ echo "VGL $(date -u +%FT%T.%3NZ) $*" | tee -a $D/session.log; }
[ "$(pgrep -c qemu-system)" = 0 ] || { L "REFUSED: qemu running"; exit 4; }
[ "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ ] || { L "REFUSED: group 11 not DMA-FQ"; exit 3; }
[ "${REUSE:-0}" = 1 ] || bash $T/vfio_gl_reference.sh detach 2>&1 | tee -a $D/session.log
bash $T/vfio_gl_reference.sh run $N $([ "${REUSE:-0}" = 1 ] && echo reuse || echo fresh) 2>&1 | tee -a $D/session.log
QPID=$(cat $R/qemu.pid); L "qemu pid $QPID"
for i in $(seq 90); do kill -0 $QPID 2>/dev/null || { L "qemu died"; exit 2; }; G qga-ping >/dev/null 2>&1 && break; sleep 2; done
L "qga up after $i tries"
for t in 1 2 3 4 5; do G qga-exec net.exe user vast $PW >/dev/null 2>&1 && break; sleep 5; done; L "password set (tries $t)"
for i in $(seq 60); do G qga-exec powershell.exe -NoProfile -Command 'if (Get-Process LogonUI -ErrorAction SilentlyContinue) {"LOGONUI"}' 2>/dev/null | grep -q LOGONUI && break; sleep 4; done
L "LogonUI seen after $i probes; waiting 15 s"; sleep 15
key spc; sleep 3; for c in k f s i g n 7; do key $c; done; key ret; L "sign-in keys sent"
for i in $(seq 40); do G qga-exec cmd.exe /c quser 2>/dev/null | grep -qi 'vast.*Active' && break; sleep 4; done
L "explorer as vast after $i probes"; sleep 20
Q cmd blockdev-add '{"driver":"raw","node-name":"kfapps_exfat","read-only":true,"file":{"driver":"file","filename":"'$W'/appmatrix/image/kfapps_exfat.img","read-only":true}}' > $D/attach.txt 2>&1
Q cmd device_add '{"driver":"usb-storage","id":"kfappsusb2","bus":"xhci.0","drive":"kfapps_exfat","removable":true}' >> $D/attach.txt 2>&1
sleep 8; G qga-exec powershell.exe -NoProfile -Command 'Get-Volume | Format-Table DriveLetter,FileSystemLabel,FileSystem,Size | Out-String; Test-Path D:\tools\kf_glgears.exe' 2>&1 | tr -d '\r' | tee $D/vols.txt
L "READY (VM left running)"
