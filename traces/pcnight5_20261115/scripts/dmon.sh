#!/bin/bash
# host-side: nvidia-smi dmon for the benchmark window (kf3 mode only: under VFIO the host driver is unloaded)
exec nvidia-smi dmon -s pucvmet -d 1 -o DT > "$1" 2>&1
