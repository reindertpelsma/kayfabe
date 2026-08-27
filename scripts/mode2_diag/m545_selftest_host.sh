#!/usr/bin/env bash
# m545_selftest_host.sh — ON HOST. M5.45 host-channel SELF-TEST: build QEMU, fresh
# restart the VM with NVKVM_SELFTEST=1 (the self-test runs at device realize, NO guest
# involvement), sample host GPU util during the test window, then print the decisive
# self-test evidence (sem landed? host GP_GET advanced? util? Xid?).
set -u
QLOG=/tmp/m0_qemu.log

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_SELFTEST=1)"
pkill -9 qemu-system 2>/dev/null; sleep 3
nohup env NVKVM_SELFTEST=1 bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 2; pgrep qemu-system >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> host GPU util sampler (30s, 1s period — self-test runs at realize)"
for t in $(seq 1 30); do
  echo "util_t${t}s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"
  grep -aq "M5.45 SELFTEST VERDICT" "$QLOG" 2>/dev/null && [ "$t" -gt 8 ] && break
  sleep 1
done > /tmp/m545_util.log 2>&1

echo ""; echo "============ M5.45 SELF-TEST EVIDENCE ============"
echo "--- self-test trace (all M5.45 lines) ---"
grep -a "M5.45" "$QLOG"
echo "--- supporting MEMTEST/isolate lines ---"
grep -aE "M5.1: host isolate|MEMTEST: \*\*\*" "$QLOG" | head -3
echo "--- host GPU util during window ---"
grep -oE "[0-9]+ %" /tmp/m545_util.log | sort -rn | uniq -c | head -5
echo "--- host dmesg Xid (during/after) ---"
sudo dmesg 2>/dev/null | grep -iE "xid" | tail -5
echo "============ END ============"
