#!/bin/bash
# A single writer owns the QEMU source archive; both installed binaries are revision-private.
set -euo pipefail
bash /root/sweep2/build_measured.sh
if bash /root/sweep2/build_b1.sh; then
 echo OBSERVED_B1_PREPARED
else
 echo B1_BUILD_UNAVAILABLE_NO_B1_VM
fi
