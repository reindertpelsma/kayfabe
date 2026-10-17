#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /root/ada2build-payload
exec >/root/prov/ada2build_payload.log 2>&1
trap 'rc=$?; echo "ADA2BUILD_PAYLOAD_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "ADA2BUILD_PAYLOAD_START $(date -u -Is)"
cd /root/ada2build-payload
curl -fL --retry 3 --max-time 300 https://developer.nvidia.com/downloads/vulkan-beta-58151-windows -o driver58151.exe
printf '%s\n' 'cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a  driver58151.exe' | sha256sum -c -
test "$(stat -c %s driver58151.exe)" = 910810544
# Wait for the setup package transaction; installing a CPU-only ISO builder does not launch a GPU VM.
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=300 install -y -qq p7zip-full genisoimage >/root/prov/ada2build_payload_deps.log 2>&1
7z t driver58151.exe >/root/prov/ada2build_payload_crc.log
mkdir -p nvidia58151
7z x -y -onvidia58151 driver58151.exe >/root/prov/ada2build_payload_extract.log
# This CD is public package/test payload only, no passwords or native capture data.
for f in cup2.exe cup3.exe cup8.exe kf_dxprobe.exe; do test -s "$f"; done
genisoimage -quiet -J -R -iso-level 3 -V KFPAYLOAD -o /root/ada2build-payload.iso .
sha256sum driver58151.exe nvidia58151/setup.exe cup2.exe cup3.exe cup8.exe kf_dxprobe.exe /root/ada2build-payload.iso >/root/prov/ada2build_payload_hashes.txt
stat -c 'PAYLOAD_ISO bytes=%s' /root/ada2build-payload.iso
echo "ADA2BUILD_PAYLOAD_READY $(date -u -Is)"
