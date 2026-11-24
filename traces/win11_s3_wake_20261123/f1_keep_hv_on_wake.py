#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""f1_keep_hv_on_wake.py QEMU_SRC_COPY -- the F1 experiment of PREREGISTRATION.md (STATUS: RESEARCH, 2026-11-23).

Rewrites hw/i386/pc.c of a COPY of the pinned QEMU 10.2.4 tree so that `pc_machine_wakeup` keeps every vCPU's Hyper-V
synthetic MSR state (CPUX86State msr_hv_hypercall .. msr_hv_tsc_emulation_status) across the RESET_TYPE_WAKEUP reset:
the state is read from KVM (cpu_synchronize_all_states), the reset runs, the saved block is put back into env, and
cpu_synchronize_all_post_reset writes it to KVM. Architectural CPU state is reset as before. Evidence tool only: the
product build (scripts/bench/build_kf3.sh) never runs it.
"""
import sys
from pathlib import Path

OLD = """static void pc_machine_wakeup(MachineState *machine)
{
    cpu_synchronize_all_states();
    pc_machine_reset(machine, RESET_TYPE_WAKEUP);
    cpu_synchronize_all_post_reset();
}
"""
NEW = """/* kayfabe F1 (experiment, 2026-11-23): a guest S3 wake keeps the Hyper-V synthetic state. */
#define KF_HV_LO offsetof(CPUX86State, msr_hv_hypercall)
#define KF_HV_LEN (offsetof(CPUX86State, msr_hv_tsc_emulation_status) + sizeof(uint64_t) - KF_HV_LO)
static void pc_machine_wakeup(MachineState *machine)
{
    CPUState *cs;
    size_t n = 0, i = 0;
    uint8_t *save;

    cpu_synchronize_all_states();
    CPU_FOREACH(cs) {
        n++;
    }
    save = g_malloc(n * KF_HV_LEN);
    CPU_FOREACH(cs) {
        memcpy(save + KF_HV_LEN * i++, (uint8_t *)&X86_CPU(cs)->env + KF_HV_LO, KF_HV_LEN);
    }
    pc_machine_reset(machine, RESET_TYPE_WAKEUP);
    i = 0;
    CPU_FOREACH(cs) {
        memcpy((uint8_t *)&X86_CPU(cs)->env + KF_HV_LO, save + KF_HV_LEN * i++, KF_HV_LEN);
    }
    g_free(save);
    fprintf(stderr, "kayfabe-F1: S3 wake kept the Hyper-V MSR block of %zu vCPU(s)\\n", n);
    cpu_synchronize_all_post_reset();
}
"""


def main():
    pc = Path(sys.argv[1]) / "hw" / "i386" / "pc.c"
    t = pc.read_text()
    if NEW in t:
        print("F1 already applied")
        return
    if t.count(OLD) != 1:
        sys.exit("pc_machine_wakeup not found exactly once: not the pinned QEMU 10.2.4?")
    pc.write_text(t.replace(OLD, NEW))
    print(f"F1 applied to {pc}")


if __name__ == "__main__":
    main()
