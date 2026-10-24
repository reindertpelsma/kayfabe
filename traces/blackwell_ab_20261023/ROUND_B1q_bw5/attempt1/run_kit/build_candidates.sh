#!/bin/bash
# A single writer owns the QEMU source archive; both installed binaries are revision-private.
set -euo pipefail
bash /root/sweep2/build_measured.sh
# BW5 B1p: the pinned fix revision replaces the B1 build; every B1p arm runs it.
if bash /root/sweep2/build_fix.sh; then
 echo OBSERVED_FIX_PREPARED
else
 echo FIX_BUILD_UNAVAILABLE_NO_FIX_VM
fi
