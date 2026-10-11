#!/usr/bin/env bash
# Reproducible fresh-guest media; run only with the GPU idle under its bench lock.
set -euo pipefail
if [[ ${KF_LOCKED:-0} != 1 ]]; then
 exec flock -o /tmp/kayfabe-fastguest.lock env KF_LOCKED=1 bash "$0" "$@"
fi
TREE=${1:?pass a frozen harness worktree}
REV=$(git -C "$TREE" rev-parse --short=8 HEAD)
W=/var/lib/kf-windows-20261005
BASE=$W/appmatrix/image/kfapps_exfat.img
ADD=$W/codex-supplement-20261011/dl
OUT=$W/codex-supplement-20261011/full-image-$REV
SRC=$OUT/source-mount
DST=$OUT/dest-mount
IMG=$OUT/kfapps_supplement.img
AM=$TREE/scripts/bench/windows/appmatrix
! pgrep qemu-system
[[ $(df -B1 --output=avail /var/lib | tail -1) -gt 161061273600 ]]
[[ ! -e $OUT ]]
mkdir -p "$SRC" "$DST"
cleanup() {
 rc=$?; trap - EXIT
 if mountpoint -q "$DST"; then umount "$DST"; fi
 if mountpoint -q "$SRC"; then umount "$SRC"; fi
 echo "FULL_MEDIA_EXIT=$rc harness=$REV at=$(date -Is)"
}
trap cleanup EXIT
echo "FULL_MEDIA_START harness=$REV at=$(date -Is)"
mount -o loop,ro "$BASE" "$SRC"
truncate -s 16G "$IMG"
mkfs.exfat -L KFAPPS "$IMG"
mount -o loop "$IMG" "$DST"
cp -rL "$SRC/." "$DST/"
python3 - "$AM" "$ADD" "$DST" "$OUT" "$REV" <<'PY'
import datetime,hashlib,json,pathlib,shutil,sys
am,add,dst,out=map(pathlib.Path,sys.argv[1:5]);rev=sys.argv[5]
man=json.loads((am/'manifest.json').read_text())
def digest(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
for pkg in man['packages']:
 if pkg['id'] not in ('hf_wheels','hf_model','cuda_nvrtc'):continue
 target=dst/'pkg'/pkg['id'];target.mkdir(parents=True,exist_ok=True)
 for f in pkg['files']:
  src=add/pkg['id']/f['name']
  assert src.stat().st_size==f['size'] and digest(src)==f['sha256'],str(src)
  shutil.copyfile(src,target/f['name'])
for src in (am/'guest').rglob('*'):
 if src.is_file() and src.suffix in ('.ps1','.py','.html'):
  target=dst/'guest'/src.relative_to(am/'guest')
  target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,target)
shutil.copyfile(am/'apps.json',dst/'apps.json')
tools=pathlib.Path('/var/lib/kf-windows-20261005/codex-matrix-focus-tools-20261011')
for name,want in {
 'kf_dxprobe.exe':'3c6ededa77430a18a13321eb72fdca7f60282f8d12ddc147a7e7df1284832c5f',
 'kf_cuda_control.exe':'3906f520c10e272215d9d7d79a90ce9a2042581923a4717db90be312ab3a67c8',
}.items():
 src=tools/name;assert digest(src)==want;shutil.copyfile(src,dst/'tools'/name)
files={str(p.relative_to(dst)):dict(sha256=digest(p),size=p.stat().st_size)
       for p in sorted(dst.rglob('*')) if p.is_file() and p.name!='KFAPPS.ID'}
ident=hashlib.sha256(json.dumps(files,sort_keys=True).encode()).hexdigest()
(dst/'KFAPPS.ID').write_text(ident+'\n')
man['files']=files
man['image'].update(file='kfapps_supplement.img',format='exfat',identity=ident,
 built_utc=datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
 built_with='build_full_media.sh',harness_revision=rev)
(out/'manifest.json').write_text(json.dumps(man,indent=1)+'\n')
print('FULL_MEDIA_FILES',len(files),'IDENTITY',ident,flush=True)
PY
umount "$DST"
umount "$SRC"
python3 - "$OUT" <<'PY'
import hashlib,json,pathlib,sys
out=pathlib.Path(sys.argv[1]);img=out/'kfapps_supplement.img'
with img.open('rb') as f:h=hashlib.file_digest(f,'sha256').hexdigest()
man=json.loads((out/'manifest.json').read_text());man['image'].update(sha256=h,size=img.stat().st_size)
(out/'manifest.json').write_text(json.dumps(man,indent=1)+'\n')
(out/'kfapps_supplement.img.sha256').write_text(h+'  kfapps_supplement.img\n')
print('FULL_MEDIA_SHA256',h,flush=True)
PY
python3 "$AM/appdisk.py" verify-image --iso "$IMG" --manifest "$OUT/manifest.json" --mount --whole
