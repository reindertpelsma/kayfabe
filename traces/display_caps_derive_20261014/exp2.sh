#!/bin/bash
# exp.sh NAME "DROP flags" "ADD flags" [bin=probe1] [secs=120]  — one Windows boot of the phase1 overlay on this host (own run dir)
set -uo pipefail
N=$1; DROP=${2:-}; ADD=${3:-}; BIN=${4:-probe1}; SECS=${5:-120}
W=/data/claude-caps/win; E=/data/claude-caps/exp/$N
[ -e $E ] && { echo "exists $E"; exit 2; }
mkdir -p $E
cp --reflink=auto $W/run/windows.qcow2 $W/run/OVMF_VARS.fd $E/
cd /data/claude-caps/wt
export WIN_DIR=$W WIN_RUN=$E QBIN=/data/claude-caps/bins/$BIN/qemu-system-x86_64 WIN_SMP=4 WIN_RAM_MB=4096 WIN_FB_MB=2048 QGA_WAIT=420 PHASE2_SECONDS=$SECS WIN_FLAGS_DROP="$DROP" WIN_FLAGS_ADD="$ADD"
echo "EXP_START $N drop=[$DROP] add=[$ADD] bin=$BIN $(date -Is)" > $E/summary.txt
bash scripts/bench/diesweep/die_run.sh phase2
kill $(cat $E/qpid) 2>/dev/null; sleep 4; kill -9 $(cat $E/qpid) 2>/dev/null; true
Q=$(ls -1t $E/qemu*.log | head -1)
{
echo "== counts"
for k in "GSP phase Running -> Suspending" "USER_RING_REFUSED" "display: STALL" "life ENDED"; do echo "$k: $(grep -ac "$k" $Q)"; done
echo "== display"
grep -a "BISECT\|PROBE KF3_DISPLAY_CAPS\|armed its first head\|console shows head\|no armed head has scanned\|life ENDED" $Q | cut -c1-260 | head -12
grep -a "updates completed" $Q | head -3 | cut -c1-120
echo "== windows initialised (channels allocated: Window)"; grep -ac "ChannelAllocated { kind: Window," $Q
} >> $E/summary.txt 2>&1
last=$(ls -1t $E/shots/*.ppm.gz | head -1); zcat $last | convert ppm:- -resize 960x $E/final.png
echo "EXP_DONE $N $(date -Is)" >> $E/summary.txt
