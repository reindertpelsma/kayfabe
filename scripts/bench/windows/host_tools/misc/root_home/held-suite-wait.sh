#!/bin/bash
export PATH=/root/.cargo/bin:$PATH
while [ "$(pgrep -c qemu-system-x86)" != 0 ]; do sleep 10; done
cd /var/lib/kf-windows-20261005/kayfabe-heldhole
exec flock -w 14400 -o /tmp/kayfabe-fastguest.lock env KF_LOCK=/tmp/held-suite-inner.lock KF_DEVICE=kf3 bash -c "while [ \$(pgrep -c qemu-system-x86) != 0 ]; do sleep 5; done; bash scripts/fastguest/fast_suite.sh heldhole3 180"
