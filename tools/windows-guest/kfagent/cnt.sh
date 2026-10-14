R=/var/lib/kf-windows-20261005/boundary-kayfabe-2990
for k in "GSP phase Running -> Suspending" "USER_RING_REFUSED" "USER_RING_SCAN" "REFUSED fn76" "display: STALL" "BLACK:"; do echo "$k: $(grep -c "$k" $R/qemu.log)"; done
