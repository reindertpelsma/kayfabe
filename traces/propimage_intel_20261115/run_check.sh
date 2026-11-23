#!/bin/bash
# usage: run_check.sh <name> [extra propimage_check args...]; runs the SCRATCH propimage_check (host check relaxed) on the box.
# start marker and exit line in /root/prop/<name>.log; never boots the base image (the check makes an overlay)
N=$1; shift
S=/root/scratch/windows
export KF_INTEL_EXPERIMENT=1 SWTPM=/opt/tpm/bin/swtpm LD_LIBRARY_PATH=/opt/tpm/lib
[ -n "${KF_INTEL_CPU_ARG:-}" ] && export KF_INTEL_CPU_ARG
mkdir -p /root/prop; L=/root/prop/$N.log
echo "START $(date -u +%FT%TZ) name=$N cpu_arg=${KF_INTEL_CPU_ARG:-<profile>} args=$*" > $L
rm -rf /root/prop/work_$N /root/prop/out_$N
python3 -B -I $S/propimage_check.py ${KF_ZIP:+--zip $KF_ZIP} --dir /root/prop/unpacked --work /root/prop/work_$N --out /root/prop/out_$N \
  --qemu "$QEMU" --flags $S/kf3_flags.sample.env --broker /root/kayfabe/broker/nvkvm-display-broker --gop-efi /root/kf-gop.signed.efi "$@" >> $L 2>&1
echo "EXIT rc=$? $(date -u +%FT%TZ)" >> $L
