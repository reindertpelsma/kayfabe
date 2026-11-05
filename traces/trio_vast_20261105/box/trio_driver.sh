#!/bin/bash
# box-side driver for lane trio (2026-11-05): merge check (Linux solo baseline) with the Windows install beside it
# after FG_RC=0, then windows_gate.sh (Windows solo baseline + the overlay), then trio_stage.sh.
# Log: /root/prov/driver.log, STEP lines and a last DRIVER_EXIT line.
exec >> /root/prov/driver.log 2>&1
echo "DRIVER_START $(date -u -Is)"
trap 'echo "DRIVER_EXIT rc=$? $(date -u -Is)"' EXIT
B=claude/trio-vast-20261105
until [ -d /root/kayfabe/.git ] && [ -f /root/prov/READY -o -n "$(grep -s "^EXIT" /root/prov/prov.log)" ]; do sleep 20; done
cd /root/kayfabe || exit 1
REV=$(git rev-parse --short=8 HEAD)
echo "REV=$REV"
until [ -f /root/prov/READY ] || grep -q '^EXIT' /root/prov/prov.log 2>/dev/null; do sleep 20; done
[ -f /root/prov/READY ] || { echo "STEP prov NOT_READY"; exit 1; }
echo "STEP prov READY $(date -u -Is)"
# media fetch + apt prerequisites (no QEMU) right away
(bash scripts/bench/windows/win_vm.sh fetch > /root/prov/win_fetch.log 2>&1; echo "FETCH_EXIT rc=$? $(date -u -Is)" >> /root/prov/win_fetch.log) &
DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=300 install -y -qq ovmf swtpm swtpm-tools sbsigntool \
  python3-virt-firmware xorriso netpbm p7zip-full gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 > /root/prov/win_prereq.log 2>&1
echo "STEP prereqs rc=$? $(date -u -Is)"
CARGO_BUILD_JOBS=$(nproc) bash scripts/bench/box/merge_check.sh "$B" trio_mc &
MCPID=$!
# the install runs a QEMU: build_fast_guest.sh refuses while one runs, so start after FG_RC=0 (or the merge check's end)
until grep -q '^FG_RC=0' /root/prov/trio_mc.log 2>/dev/null || ! kill -0 "$MCPID" 2>/dev/null; do sleep 15; done
wait_fetch=0
until grep -q FETCH_EXIT /root/prov/win_fetch.log 2>/dev/null; do sleep 10; wait_fetch=$((wait_fetch+10)); done
echo "STEP fetch $(grep FETCH_EXIT /root/prov/win_fetch.log) waited=${wait_fetch}s"
smp=$(nproc); [ "$smp" -le 16 ] || smp=16
echo "STEP install START $(date -u -Is) smp=$smp"
WINVM_VM_IDENTITY=1 WINVM_SMP=$smp WINVM_INSTALL_CACHE=unsafe WINVM_SSH_CONNECT_TIMEOUT=100 WINVM_INSTALL_WAIT_S=4200 \
  WINVM_QEMU=/workspace/bench/kf3-bins/$REV/qemu-system-x86_64 \
  bash scripts/bench/windows/win_vm.sh install > /root/prov/win_install.log 2>&1
echo "STEP install EXIT rc=$? $(date -u -Is)"
wait "$MCPID"
echo "STEP merge_check done: $(grep -aE '^FAST_SUITE_PASS|^EXIT|SUMMARY' /root/prov/trio_mc.log /workspace/bench/trio_mc_suite.out 2>/dev/null | tr '\n' ' ')"
echo "STEP gate START $(date -u -Is)"
bash scripts/bench/box/windows_gate.sh trio_g /root/prov
echo "STEP gate EXIT rc=$? $(date -u -Is)"
grep -q '^WINDOWS_GATE_PASS' /root/prov/trio_g_windows.log || { echo "STEP no gate PASS: trio not started"; exit 3; }
echo "STEP trio START $(date -u -Is)"
bash scripts/bench/box/trio_stage.sh trio1 trio_g /root/prov
echo "STEP trio EXIT rc=$? $(date -u -Is)"
