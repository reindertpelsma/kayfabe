#!/bin/bash
export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/var/lib/kf-windows-20261005/target-fb8150ed
echo START $(date -u +%FT%T) > /var/lib/kf-windows-20261005/build-fb8150ed.log
cd /var/lib/kf-windows-20261005/kayfabe-fb8150ed
bash scripts/bench/build_kf3.sh /var/lib/kf-windows-20261005/irqflood-qemu/qemu-10.2.4 /var/lib/kf-windows-20261005/irqflood-qemu/qemu-build-kf3 >> /var/lib/kf-windows-20261005/build-fb8150ed.log 2>&1
echo EXIT $? $(date -u +%FT%T) >> /var/lib/kf-windows-20261005/build-fb8150ed.log
