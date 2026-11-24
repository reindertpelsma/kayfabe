#!/bin/bash
cd /root/kayfabe
# wait for the running base_595 sweep (it holds the sweep lock)
while pgrep -f "[s]weep.sh" >/dev/null; do sleep 15; done
echo "DRIVER3_START $(date -Is)"
export MIXED=
TAG=hostabi_pr_595g595 KF_REV=8a4bd4b0b9c85daf970fc4cc659eadea0ec68824 HOSTS=595.84 REF_HOST=595.84 GUESTS=595.84 DEFAULT_GUEST=595.84 RESTORE_REF=0 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_pr_595g595.out 2>&1; echo "SWEEP pr_595g595 rc=$? $(date -Is)"
TAG=hostabi_base_595g595 KF_REV=f2f009a8c5462038d8fe51e108c947a84cf0298b HOSTS=595.84 REF_HOST=595.84 GUESTS=595.84 DEFAULT_GUEST=595.84 RESTORE_REF=0 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_base_595g595.out 2>&1; echo "SWEEP base_595g595 rc=$? $(date -Is)"
TAG=hostabi_base_580 KF_REV=f2f009a8c5462038d8fe51e108c947a84cf0298b HOSTS=580.159.04 REF_HOST=580.159.04 GUESTS=580.159.04 DEFAULT_GUEST=580.159.04 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_base_580.out 2>&1; echo "SWEEP base_580 rc=$? $(date -Is)"
echo "DRIVER3_EXIT $(date -Is)"
