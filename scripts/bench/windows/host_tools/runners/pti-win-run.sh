#!/bin/bash
W=/var/lib/kf-windows-20261005
echo "PTIWIN_START $(date -Is)"
bash $W/pti-iommu_nogdm.sh identity || { echo PTIWIN_IOMMU_REFUSED; exit 3; }
KF3_REV=916355df bash $W/pti-win-cycle.sh 92 0 wait_stall sleep:30 kill
echo "PTIWIN_CYCLE_EXIT $(date -Is) qemu=$(pgrep -c qemu-system)"
bash $W/pti-iommu_nogdm.sh DMA-FQ
echo "PTIWIN_DONE $(date -Is)"
