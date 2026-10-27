#!/usr/bin/env python3
"""Mutation checks for the host-driver swap repair (2026-10-26): every mutant needs two killers.

Targets scripts/bench/provision_host_driver.sh (nouveau guard, reset-and-reload recovery, rc 8) and
scripts/bench/box/provision_full.sh (early stop on rc 8, explained NOT_READY), driven by
scripts/ci/test_vast_gpu_swap.py. Stdlib only; GPU-free.

    python3 scripts/ci/gpu_swap_mutants.py [substring-of-mutant-name]
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DRIVER = ROOT / "scripts/bench/provision_host_driver.sh"
FULL = ROOT / "scripts/bench/box/provision_full.sh"
TESTS = "test_vast_gpu_swap"

# (name, target, old text (exactly once), new text)
MUTANTS = [
    ("guard-no-udev-reload", DRIVER, "  udevadm control --reload 2>/dev/null\n", ""),
    ("guard-no-blacklist", DRIVER, "blacklist nouveau\\noptions", "options"),
    ("guard-no-modeset-off", DRIVER, "options nouveau modeset=0\\n", ""),
    ("guard-never-called", DRIVER, "# KF_GPU_SETTLE_END\nnouveau_guard\n", "# KF_GPU_SETTLE_END\n"),
    ("reset-skips-vendor-check", DRIVER, "= 0x10de ] || continue", "!= 0x10de ] || continue"),
    (
        "reset-takes-audio-function",
        DRIVER,
        'case "$(cat "$d/class" 2>/dev/null)" in 0x03*)',
        'case "$(cat "$d/class" 2>/dev/null)" in 0x0*)',
    ),
    ("reset-never-written", DRIVER, 'do kf_pci_reset "$bdf"; done', "do :; done"),
    (
        "reset-writes-zero",
        DRIVER,
        'echo 1 > "$KF_SYSFS_PCI/$1/reset"',
        'echo 0 > "$KF_SYSFS_PCI/$1/reset"',
    ),
    (
        "recovery-no-unload",
        DRIVER,
        "  systemctl stop nvidia-persistenced 2>/dev/null\n  unload_nvidia\n",
        "  systemctl stop nvidia-persistenced 2>/dev/null\n",
    ),
    (
        "recovery-keeps-nouveau",
        DRIVER,
        "nvidia_modeset nvidia nouveau; do rmmod",
        "nvidia_modeset nvidia; do rmmod",
    ),
    (
        "recovery-keeps-persistenced",
        DRIVER,
        "  systemctl stop nvidia-persistenced 2>/dev/null\n  unload_nvidia\n",
        "  unload_nvidia\n",
    ),
    (
        "recovery-no-reload",
        DRIVER,
        "  modprobe nvidia 2>/dev/null; modprobe nvidia_uvm 2>/dev/null\n",
        "  modprobe nvidia_uvm 2>/dev/null\n",
    ),
    (
        "check-trusts-exit-status",
        DRIVER,
        '  [ "$GPU_SMI_RC" -eq 0 ] && printf \'%s\\n\' "$GPU_SMI_OUT"'
        " | grep -qE '^[^,]+, [0-9]+\\.[0-9]+'\n",
        '  [ "$GPU_SMI_RC" -eq 0 ]\n',
    ),
    ("tries-default-zero", DRIVER, "KF_RECOVER_TRIES:-3", "KF_RECOVER_TRIES:-0"),
    (
        "tries-off-by-one",
        DRIVER,
        '[ "$n" -lt "$KF_RECOVER_TRIES" ] || break',
        '[ "$n" -le "$KF_RECOVER_TRIES" ] || break',
    ),
    (
        "settle-verdict-not-eight",
        DRIVER,
        "  return 8\n}\n# KF_GPU_SETTLE_END",
        "  return 1\n}\n# KF_GPU_SETTLE_END",
    ),
    (
        "diag-tail-too-long",
        DRIVER,
        "grep -a NVRM | tail -n 20 | cut -c1-200\n  return 8",
        "grep -a NVRM | tail -n 200 | cut -c1-200\n  return 8",
    ),
    (
        "diag-no-nvrm-filter",
        DRIVER,
        "grep -a NVRM | tail -n 20 | cut -c1-200\n  return 8",
        "tail -n 20 | cut -c1-200\n  return 8",
    ),
    (
        "diag-no-line-cut",
        DRIVER,
        "grep -a NVRM | tail -n 20 | cut -c1-200\n  return 8",
        "grep -a NVRM | tail -n 20\n  return 8",
    ),
    (
        "call-exits-one",
        DRIVER,
        'exit 8; }\necho "DRIVER_SWAP_DONE rc=0',
        'exit 1; }\necho "DRIVER_SWAP_DONE rc=0',
    ),
    (
        "call-says-rc-zero-on-failure",
        DRIVER,
        "DRIVER_SWAP_DONE rc=8 (GPU",
        "DRIVER_SWAP_DONE rc=0 (GPU",
    ),
    (
        "call-ignores-settle",
        DRIVER,
        'gpu_settle || { echo "DRIVER_SWAP_DONE rc=8',
        'true || { echo "DRIVER_SWAP_DONE rc=8',
    ),
    ("gate-never-fires", FULL, 'if [ "$DRIVER_RC" = 8 ]; then', 'if [ "$DRIVER_RC" = 99 ]; then'),
    (
        "gate-fires-on-any-failure",
        FULL,
        'if [ "$DRIVER_RC" = 8 ]; then',
        'if [ "$DRIVER_RC" != 0 ]; then',
    ),
    ("gate-without-diagnostics", FULL, '  not_ready_diag\n  echo "EXIT', '  echo "EXIT'),
    ("ready-without-diagnostics", FULL, 'FG_RC=$FG_RC"; not_ready_diag; fi', 'FG_RC=$FG_RC"; fi'),
    ("diag-no-smi-status", FULL, '  echo "NVIDIA_SMI_RC=$rc"\n', ""),
    (
        "diag-tail-too-long-full",
        FULL,
        'tail -n 20 | cut -c1-200\n  echo "NVRM_TAIL_END"',
        'tail -n 200 | cut -c1-200\n  echo "NVRM_TAIL_END"',
    ),
    (
        "diag-no-line-cut-full",
        FULL,
        'tail -n 20 | cut -c1-200\n  echo "NVRM_TAIL_END"',
        'tail -n 20\n  echo "NVRM_TAIL_END"',
    ),
    (
        "ready-accepts-any-driver",
        FULL,
        'if [ "$V" = "${KF_HOST_DRIVER:-580.159.04}" ] &&',
        'if [ -n "$V" ] &&',
    ),
    ("ready-ignores-kf3-rc", FULL, '&& [ "$KF3_RC" = 0 ] &&', "&&"),
    (
        "not-ready-line-renamed",
        FULL,
        'echo "NOT_READY driver=$V KF3_RC',
        'echo "NOT_READY drv=$V KF3_RC',
    ),
]


def run(target, mutated):
    var = "KF_PROVISION_DRIVER" if target == DRIVER else "KF_PROVISION_FULL"
    env = dict(os.environ, **{var: str(mutated)})
    p = subprocess.run(
        [sys.executable, "-m", "unittest", "-v", TESTS],
        cwd=HERE,
        env=env,
        capture_output=True,
        text=True,
        timeout=900,
    )
    failed = []
    for line in (p.stdout + p.stderr).splitlines():
        if line.startswith(("FAIL: ", "ERROR: ")):
            failed.append(line.split()[1])
    return failed


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rows, bad = [], 0
    with tempfile.TemporaryDirectory(prefix="kf-gpuswap-") as directory:
        for target in (DRIVER, FULL):
            work = Path(directory) / ("base-" + target.name)
            work.write_text(target.read_text())
            baseline = run(target, work)
            if baseline:
                print("BASELINE RED for %s: %s" % (target.name, baseline), file=sys.stderr)
                return 1
        for name, target, old, new in MUTANTS:
            if only not in name:
                continue
            source = target.read_text()
            assert source.count(old) == 1, (name, source.count(old))
            work = Path(directory) / ("mut-" + name + "-" + target.name)
            work.write_text(source.replace(old, new))
            killers = run(target, work)
            rows.append({"mutant": name, "killers": killers, "count": len(killers)})
            print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
            bad += len(killers) < 2
    print(json.dumps({"cases": rows}, indent=2))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
