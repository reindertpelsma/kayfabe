#!/bin/bash
exec >>/root/prov/q2.log 2>&1
echo "Q2_START $(date -u -Is)"
until grep -q CTL2_READY /root/prov/bwr_ctl2.log && grep -q 'FIXBUILD_EXIT name=fix5' /root/prov/bwr_fix5_build.log 2>/dev/null; do
  grep -q 'NATIVE_STOP' /root/prov/bwr_ctl2.log && { echo Q2_NATIVE_STOP; exit 2; }
  sleep 10; done
grep -q 'FIXBUILD_READY name=fix5' /root/prov/bwr_fix5_build.log || { echo Q2_NO_FIX5; exit 3; }
pan(){ grep -q GUEST_PANICKED /workspace/bwr-win/kfwin/logs/$1_run.log 2>/dev/null; }
bash /root/bwr/seq.sh fix3_off_1:fix3:off; pan fix3_off_1 && bash /root/bwr/collect.sh fix3_off_1
ARM_HOLD=240 bash /root/bwr/seq.sh fix4_off_1:fix4:off; pan fix4_off_1 && bash /root/bwr/collect.sh fix4_off_1
ARM_HOLD=180 bash /root/bwr/seq.sh fix5_on_1:fix5:on; pan fix5_on_1 && bash /root/bwr/collect.sh fix5_on_1
ARM_HOLD=240 bash /root/bwr/seq.sh fix4_off_2:fix4:off; pan fix4_off_2 && bash /root/bwr/collect.sh fix4_off_2
ARM_HOLD=180 bash /root/bwr/seq.sh fix5_on_2:fix5:on; pan fix5_on_2 && bash /root/bwr/collect.sh fix5_on_2
echo "Q2_EXIT $(date -u -Is)"
