#!/usr/bin/env bash
# gopfinal_runs.sh <rev> <suffix> — at <rev>: build kf3; B0 (gop=off); B1 (gop=on, hook.sh, timed shots);
# B5 (gop=on, DISPLAY_HOOK=unload_hook, KF3_DISPLAY_TRACE=1). Start marker, per-lane rc, exit marker.
# Shots are converted to PNG here (PIL); nothing executable is produced for copying back.
set -uo pipefail
REV=${1:?rev}; SFX=${2:-g}
export PATH=$HOME/.cargo/bin:$PATH
B=/workspace/bench
echo "GOPFINAL_START rev=$REV sfx=$SFX $(date -Is)"
cd /root/kayfabe || { echo "GOPFINAL_EXIT rc=90"; exit 90; }
git fetch -q origin && git checkout -q --detach "$REV" || { echo "GOPFINAL_EXIT rc=91"; exit 91; }
echo "HEAD $(git rev-parse --short=8 HEAD) dirty=[$(git status --porcelain --untracked-files=no | wc -l)]"
bash scripts/bench/build_kf3.sh $B/qemu-10.2.4 $B/qemu-build-kf3 > $B/gopfinal_build_$SFX.log 2>&1
rc=$?; echo "BUILD_RC=$rc"; tail -2 $B/gopfinal_build_$SFX.log
[ $rc -eq 0 ] || { echo "GOPFINAL_EXIT rc=$rc build"; exit $rc; }
grep -a -o 'embedded boot-display GOP driver[^"]*' $B/gopfinal_build_$SFX.log | head -1
shot() { python3 - "$1" "$2" <<PY
import socket,sys,time
s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); time.sleep(0.2); s.recv(65536)
s.sendall(("screendump %s kf0\n" % sys.argv[2]).encode()); time.sleep(1.0); print(s.recv(65536).decode(errors="replace").strip()[-120:])
PY
}
topng() { # topng <dir>: every .ppm -> .png beside it (PIL), then a contact sheet of the t*.png
python3 - "$1" <<'PY'
import sys, os, glob
from PIL import Image, ImageDraw
d = sys.argv[1]
for p in sorted(glob.glob(os.path.join(d, "*.ppm"))):
    try:
        Image.open(p).save(p[:-4] + ".png", optimize=True)
    except Exception as e:
        print("PNG_FAIL", p, e)
ts = sorted(glob.glob(os.path.join(d, "t*.png")))
if ts:
    w, h = 384, 216
    cols = 5
    rows = (len(ts) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * w, rows * (h + 18)), "white")
    dr = ImageDraw.Draw(sheet)
    for i, p in enumerate(ts):
        im = Image.open(p).convert("RGB").resize((w, h))
        x, y = (i % cols) * w, (i // cols) * (h + 18)
        sheet.paste(im, (x, y + 18))
        dr.text((x + 4, y + 2), os.path.basename(p)[:-4] + " s", fill="black")
    sheet.save(os.path.join(d, "contact_sheet.png"), optimize=True)
print("PNG_DONE", len(glob.glob(os.path.join(d, "*.png"))))
PY
}
# --- B0: OVMF, gop=off
KF_FIRMWARE=ovmf DISPLAY_HOLD_S=20 KF_DEVICE=kf3 bash scripts/bench/display/lane.sh b0$SFX > $B/gopfinal_b0$SFX.log 2>&1
echo "B0_LANE_RC=$?"
grep -a -E "DISPLAY_(BOOT_VGA|PATTERN_MATCH|KFDISP_FLIPS|CONNECTED|LANE_EXIT|HOST_XID|BOOT_HANDOFF)|FAILED" $B/gopfinal_b0$SFX.log | cut -c1-220
echo "B0_SEED_LINES=$(grep -ac "boot display seed\|physical view\|BAR1 boot framebuffer" $B/run_b0${SFX}_qemu.log)"
echo "B0_CONSOLE_SHOWS=$(grep -ac 'the console shows' $B/run_b0${SFX}_qemu.log) B0_BLACK=$(grep -ac 'the console shows BLACK' $B/run_b0${SFX}_qemu.log)"
grep -a 'the console shows' $B/run_b0${SFX}_qemu.log | cut -c1-200 | head -12
# --- B1: OVMF, gop=on, timed shots from the moment the monitor socket appears
S=$B/display/b1${SFX}shots; mkdir -p $S; rm -f $S/*.ppm $S/*.png
KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on DISPLAY_HOLD_S=20 KF_DEVICE=kf3 bash scripts/bench/display/lane.sh b1$SFX > $B/gopfinal_b1$SFX.log 2>&1 &
LP=$!
for _ in $(seq 1 120); do [ -S $B/run_b1$SFX.mon ] && break; sleep 0.5; done
t0=$(date +%s.%N)
for t in 1 2 4 6 9 12 16 20 25 30 40 45 48 50 52 54 56 58 60 65 75 90; do
  while [ "$(echo "$(date +%s.%N) - $t0 < $t" | bc)" = 1 ]; do sleep 0.2; done
  [ -S $B/run_b1$SFX.mon ] && shot "$B/run_b1$SFX.mon" "$S/t$(printf %03d "$t").ppm" > /dev/null 2>&1
done
wait $LP; echo "B1_LANE_RC=$?"
topng $S
grep -a -E "DISPLAY_(BOOT_VGA|PATTERN_MATCH|KFDISP_FLIPS|CONNECTED|LANE_EXIT|HOST_XID|BOOT_HANDOFF)|FAILED" $B/gopfinal_b1$SFX.log | cut -c1-220
grep -a -E "boot display ON|option ROM registered|display: boot layer|GET_GSP_STATIC_INFO|boot display seed|armed its first head|cannot preserve|physical view|the console shows|lines logged" $B/run_b1${SFX}_qemu.log | cut -c1-240 | head -40
# --- B5: OVMF, gop=on, the unload arms
KF3_DISPLAY_TRACE=1 KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on DISPLAY_HOOK=unload_hook KF_DEVICE=kf3 bash scripts/bench/display/lane.sh b5$SFX > $B/gopfinal_b5$SFX.log 2>&1
echo "B5_LANE_RC=$?"
grep -a -E "^DISPLAY_(B5_JUDGE|B5_VERDICT|BOOT_HANDOFF|LANE|HOST_XID|FLIP)" $B/gopfinal_b5$SFX.log | cut -c1-260
topng $B/display/b5$SFX
echo "GOPFINAL_EXIT rc=0 $(date -Is)"
