#!/bin/bash
# hold.sh TAG [APPS] : one boot of propimage_check.py on the unpacked sealed image that STAYS UP after the verify step until /workspace/img/release_TAG appears
# (--hold-until-file, 3 h at most); drive.py works on the guest meanwhile. Runs ON the box, from the checkout. Start marker /workspace/img/TAG.start, exit line TAG.exit.
# With APPS (comma list, default gpudiag) the harness runs those rows after the release file appears.
set -u
TAG=$1; APPS=${2:-gpudiag}
CK=${CK:-/root/kayfabe}
cd "$CK"
IMG=/workspace/img
QEMU=$(ls -d /workspace/bench/kf3-bins/*/qemu-system-x86_64 | head -1)
export KF_PS_MIN=${KF_PS_MIN:-300} KF_SSH_WAIT=${KF_SSH_WAIT:-1500} KF_WIN=$CK/scripts/bench/windows SWTPM=/opt/tpm/bin/swtpm LD_LIBRARY_PATH=/opt/tpm/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
# stamp for hold_run.py: written only after unpack.sh's verified unpack finished rc=0
if [ ! -f $IMG/unpacked/.verified ] && grep -q "UNPACK_EXIT rc=0" $IMG/unpack.log; then
  python3 -B -I -c "
import json,os,time
m=json.load(open('$IMG/unpacked/manifest.json'))['members']['image.qcow2']
json.dump({'sha256':m,'size':os.path.getsize('$IMG/unpacked/image.qcow2'),'at':time.strftime('%FT%TZ',time.gmtime())},open('$IMG/unpacked/.verified','w'))"
fi
echo "START $TAG $(date -u +%FT%TZ) apps=$APPS rev=$(git rev-parse --short HEAD) qemu=$QEMU driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" > $IMG/$TAG.start
rm -f $IMG/$TAG.exit $IMG/release_$TAG
setsid nohup bash -c "bash scripts/bench/host_xid_watch.sh $TAG -- python3 -B -I /workspace/p2/adobe_part2/hold_run.py --dir $IMG/unpacked --work $IMG/work/$TAG --out $IMG/out/$TAG --qemu $QEMU --flags scripts/bench/windows/kf3_flags.sample.env --broker $CK/broker/nvkvm-display-broker --gop-efi /workspace/p2/kf-gop.signed.efi --display broker --app-set b --data-iso /workspace/p2/test-assets.iso --apps $APPS --fb-mb 8192 --ram-mb 16384 --audio --hold-until-file $IMG/release_$TAG > $IMG/$TAG.log 2>&1; echo \"EXIT rc=\$? \$(date -u +%FT%TZ)\" > $IMG/$TAG.exit" >/dev/null 2>&1 < /dev/null &
echo launched $TAG
