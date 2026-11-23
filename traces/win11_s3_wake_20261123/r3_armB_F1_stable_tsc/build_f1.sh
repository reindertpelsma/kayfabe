set -euo pipefail
B=/workspace/bench
rm -rf $B/qemu-10.2.4-f1 && cp -a $B/qemu-10.2.4 $B/qemu-10.2.4-f1
python3 /root/s3/f1_keep_hv_on_wake.py $B/qemu-10.2.4-f1
mkdir -p $B/qemu-build-f1 && cd $B/qemu-build-f1
CONF=$(cat $B/qemu-build-kf3/.kf3-configure)
$B/qemu-10.2.4-f1/configure $CONF >/dev/null
nice -n 19 ninja -j8 qemu-system-x86_64
echo F1_BUILT $(sha256sum qemu-system-x86_64 | cut -c1-16)
