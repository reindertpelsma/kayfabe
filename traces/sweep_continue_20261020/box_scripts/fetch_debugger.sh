#!/bin/bash
set -euo pipefail
exec >/root/prov/public_debugger_fetch.log 2>&1
trap 'rc=$?; echo "PUBLIC_DEBUGGER_FETCH_EXIT rc=$rc $(date -u -Is)"' EXIT
mkdir -p /root/sweep2/windbg_pkg
cd /root/sweep2/windbg_pkg
curl -fLsS --max-time 30 https://aka.ms/windbg/download -o windbg.appinstaller
url=$(python3 - <<'PY'
import xml.etree.ElementTree as E,urllib.parse
r=E.parse('windbg.appinstaller').getroot();m=next(x for x in r if x.tag.endswith('MainBundle'));u=m.attrib['Uri'];p=urllib.parse.urlparse(u);assert p.scheme=='https' and p.netloc=='windbg.download.prss.microsoft.com' and p.path.endswith('/windbg.msixbundle');print(u)
PY
)
curl -fLsS --max-time 300 "$url" -o windbg.msixbundle
sha256sum windbg.appinstaller windbg.msixbundle > /root/prov/public_debugger_download_sha256.txt
7z x -y windbg.msixbundle -obundle >/dev/null
pkg=$(find bundle -maxdepth 1 -type f -iname '*x64*.msix' | head -1)
[ -n "$pkg" ]
7z x -y "$pkg" -ox64 >/dev/null
[ -f x64/amd64/cdb.exe ]
echo PUBLIC_DEBUGGER_READY
