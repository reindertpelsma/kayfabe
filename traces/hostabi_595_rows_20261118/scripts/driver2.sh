#!/bin/bash
cd /root/kayfabe
echo "DRIVER2_START $(date -Is)"
export MIXED= GUESTS=580.159.04 REF_HOST=580.159.04
TAG=hostabi_base_595 KF_REV=f2f009a8c5462038d8fe51e108c947a84cf0298b HOSTS=595.84 RESTORE_REF=0 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_base_595.out 2>&1; echo "SWEEP base_595 rc=$? $(date -Is)"
TAG=hostabi_base_580 KF_REV=f2f009a8c5462038d8fe51e108c947a84cf0298b HOSTS=580.159.04 bash scripts/drivermatrix/sweep.sh > /root/prov/sweep_base_580.out 2>&1; echo "SWEEP base_580 rc=$? $(date -Is)"
echo "DRIVER2_EXIT $(date -Is)"
