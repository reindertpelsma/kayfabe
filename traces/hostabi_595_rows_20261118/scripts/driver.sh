#!/bin/bash
# runs the three sweeps strictly serially on the box; EXIT line at the end
cd /root/kayfabe
echo "DRIVER_START $(date -Is)"
export MIXED= GUESTS=580.159.04 REF_HOST=580.159.04
TAG=hostabi_pr_580 KF_REV=8a4bd4b0b9c85daf970fc4cc659eadea0ec68824 HOSTS=580.159.04 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_pr_580.out 2>&1; echo "SWEEP pr_580 rc=$? $(date -Is)"
TAG=hostabi_pr_595 KF_REV=8a4bd4b0b9c85daf970fc4cc659eadea0ec68824 HOSTS=595.84 RESTORE_REF=0 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_pr_595.out 2>&1; echo "SWEEP pr_595 rc=$? $(date -Is)"
echo "DRIVER_EXIT $(date -Is)"
