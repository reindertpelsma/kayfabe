#!/bin/bash
# kvmstat.sh OUTDIR : KVM exits by reason for 30 s (perf kvm stat record on the qemu pid) + per-vCPU exit counts via the kvm_exit tracepoint
O=$1; mkdir -p $O
QP=${QP:-$(sed -n "s/^QPID=//p" /var/lib/kf-windows-20261005/windows-broker-run2991.state)}
perf kvm stat record -p $QP -o $O/kvm.data -- sleep 30 > $O/record.out 2>&1
perf kvm stat report -i $O/kvm.data --event=vmexit 2>&1 | head -30 > $O/vmexit_by_reason.txt
perf kvm stat report -i $O/kvm.data --event=mmio 2>&1 | head -20 > $O/mmio_by_addr.txt
rm -f $O/kvm.data $O/kvm.data.old
