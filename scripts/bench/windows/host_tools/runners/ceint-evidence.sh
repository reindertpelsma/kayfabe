#!/bin/bash
# Host-side: stage the text evidence of the --ce-interrupt runs (filtered of per-ioctl trace noise).
set -uo pipefail
B=/var/lib/kf-windows-20261005/ceint-bench
E=/var/lib/kf-windows-20261005/ceint-evidence
rm -rf $E; mkdir -p $E
f() { grep -av 'DOORBELL-STORE\|^IOCTL \|^IOCTL-TRACE' "$1"; }
f $B/ceint_F30_suite.out                        > $E/bare30_default_arms_9925108e.out
for i in 1 2 3; do f $B/ceint_FA${i}_ce-interrupt.log > $E/bare_ce-interrupt_9925108e_run$i.log; done
f $B/ceint_FU_unpriv.log                        > $E/bare_ce-interrupt_9925108e_unprivileged_euid65534.log
f $B/ceint_runall2.log                          > $E/hardware_cycle_9925108e.log
f $B/ceint_fastF_suite.out                      > $E/guest_kf3_fast_suite_9925108e.out
f $B/fast_ceint_fastF_ce-interrupt_serial.log   > $E/guest_kf3_ce-interrupt_serial_9925108e.log
f $B/fast_ceint_fastF_ce-client_serial.log      > $E/guest_kf3_ce-client_serial_9925108e.log
grep -a 'interrupt plane\|channel plane\|BORN\|NSI\|RELAY\|GSP REFUSED fn76/0x20800301\|RETIRED' $B/fast_ceint_fastF_ce-interrupt_qemu.log | cut -c1-420 > $E/guest_kf3_ce-interrupt_qemu_key_lines_9925108e.log
echo "NSI lines in the whole qemu log: $(grep -ac 'NSI' $B/fast_ceint_fastF_ce-interrupt_qemu.log)" >> $E/guest_kf3_ce-interrupt_qemu_key_lines_9925108e.log
# the instrument seeing another tenant (bare metal, a display VM sharing the GPU)
f $B/ceint_bare1_ce-interrupt.log               > $E/bare_tenant_noise_eef8d783_first_run_quiet_window_failed.log
f $B/ceint_bareF1_ce-interrupt.log              > $E/bare_tenant_noise_6d9e6a76_control_fired_once.log
f $B/ceint_bareG1_ce-interrupt.log              > $E/bare_tenant_noise_6d9e6a76_windows_desktop_vm_present_quiet_window_never_clean.log
cp $B/nvos_event_flags_by_tag.txt               $E/
du -sh $E; ls -la $E
