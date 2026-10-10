#!/bin/bash
set -u
exec >/root/gs-test/capture.log 2>&1
say(){ echo "[$(date +%H:%M:%S)] $*"; }
D=/var/lib/docker/volumes/nvkvm-steamos-state/_data
cd /root/gs-test/repo
git checkout -q -B fix-black origin/main && git am /root/gs-test/gs_black.patch || { say "PATCH FAILED"; exit 1; }
say "on $(git log --oneline -1)"
docker compose down --remove-orphans >/dev/null 2>&1
rm -f "$D"/frame*.ppm
export NVKVM_PRESENT_CAPTURE=/var/lib/nvkvm-steamos/frame.ppm
docker compose up -d --build >/dev/null 2>&1 || { say "UP FAILED"; exit 1; }
for i in $(seq 1 90); do ./steamos-ssh true >/dev/null 2>&1 && break; sleep 10; done
say "guest ssh up"
for i in $(seq 1 60); do
  st=$(./steamos-ssh 'systemctl --user --machine=deck@ is-active gamescope-session.service' 2>/dev/null | tr -d '\r\n ')
  [ "$st" = active ] && break; sleep 5
done
say "gamescope-session: $st"
sleep 30
say "capture file:"; ls -la "$D"/frame.ppm 2>/dev/null || say "  NO CAPTURE FILE"
if [ -f "$D/frame.ppm" ]; then
python3 - "$D/frame.ppm" <<'PY'
import sys
f=open(sys.argv[1],'rb'); 
magic=f.readline().strip()
dims=f.readline().split()
maxv=f.readline().strip()
data=f.read()
n=len(data)
mean=sum(data)/n if n else 0
nz=sum(1 for b in data[::97] if b>8)
print(f"  {magic.decode()} {dims[0].decode()}x{dims[1].decode()} bytes={n}")
print(f"  mean pixel value: {mean:.2f} / 255")
print(f"  non-black samples: {nz} of {len(data[::97])}")
print("  VERDICT:", "BLACK (nothing on screen)" if mean < 2 else "HAS A PICTURE")
PY
fi
say "DONE"
