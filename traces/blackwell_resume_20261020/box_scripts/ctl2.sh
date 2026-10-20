#!/bin/bash
exec >/root/prov/bwr_ctl2.log 2>&1
echo "CTL2_START $(date -u -Is)"
cp /root/prov/bwr_bare595.log /root/prov/bwr_bare595_run1_concurrent_install.log
cp /root/prov/bwr_native_baseline.log /root/prov/bwr_native_baseline_run1.log
until grep -q '^WIN_BASE_INSTALL_EXIT' /root/prov/bwr_install.log; do sleep 10; done
grep '^WIN_BASE_INSTALL_EXIT' /root/prov/bwr_install.log
until grep -q 'FIXBUILD_EXIT name=fix4' /root/prov/bwr_fix4_build.log 2>/dev/null; do sleep 10; done
bash /root/bwr/native_baseline.sh; echo "NATIVE2_RC=$?"; grep -a 'BARE_SUITE_PASS\|NATIVE_CUDA4' /root/prov/bwr_bare595.log /root/prov/bwr_native_baseline.log
grep -q 'NATIVE_BASELINE_EXIT rc=0' /root/prov/bwr_native_baseline.log || { echo NATIVE_STOP; exit 2; }
bash /root/bwr/bind.sh; grep -a 'BIND_EXIT' /root/prov/bwr_bind.log
echo "CTL2_READY $(date -u -Is)"
