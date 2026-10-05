#!/usr/bin/env python3
"""Recreate the pinned x64 KD bundle from Microsoft SDK CABs; execute no Windows code."""
import concurrent.futures
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import urllib.request
import zipfile

BASE = ('https://download.microsoft.com/download/06fc99ac-527e-451e-a536-8866695a2e7e/'
        'KIT_BUNDLE_WINDOWSSDK_MEDIACREATION/Installers/')
CABS = {
    '0253f7df0974f9d7169b410d812a5385.cab': '313aec50bc69267d9058adbd611f20f6cae281913317f404b3417353d50ad71d',
    '34ee98a7c9420178c55f176f75c3fe10.cab': '97689af2fd9f6cf5ae6db02fb0f268d8885cffbeceb74925f835d67aa8252656',
    '4ac48dbdddbc8ce04721f519b9cf1698.cab': '3d8d67168e7b6c77b2447648bbdb16a1a01ef3489dc1cd519471812446e61d2f',
    'e10f9740446a96314a1731aa7cb4286a.cab': '1c7ced49bc2ce403df0dc80e12347e7a33214ec5545855f3d61e1416eda366c7',
    'e8bc712abeffd7c9711ee3f55d4aa99b.cab': 'a4cac3dea34790717a7de2f0981206ab1fd8ef4e07262a85650a9f017afc4880',
}
MANIFEST_SHA = '4d1cf80283e34641a1e057b1596a5ff6eca25b9992e247bba0798b29c0ec2958'
ZIP_SHA = 'a3883456c4631bb46b9091a2d3197f17984df99ba89c36c2f8e54c976052d800'


def checked(data, expected):
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise RuntimeError(f'SHA256 mismatch: expected {expected}, got {actual}')
    return data


def main():
    if len(sys.argv) != 2:
        raise SystemExit('usage: fetch-debugger.py OUTPUT_DIRECTORY (requires 7z)')
    here = pathlib.Path(__file__).resolve().parent
    out = pathlib.Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    manifest = checked((here / 'debugger-manifest.json').read_bytes(), MANIFEST_SHA)
    files = json.loads(manifest)
    cache = out / 'cache'
    cache.mkdir(exist_ok=True)

    def fetch(item):
        name, digest = item
        cab = cache / name
        if cab.exists():
            checked(cab.read_bytes(), digest)
        else:
            with urllib.request.urlopen(BASE + name, timeout=90) as response:
                data = response.read(32 * 1024 * 1024 + 1)
            checked(data, digest)
            cab.write_bytes(data)
        dest = cache / (name + '.extracted')
        dest.mkdir(exist_ok=True)
        subprocess.run(['7z', 'x', '-y', '-o' + str(dest), str(cab)],
                       check=True, stdout=subprocess.DEVNULL)

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as workers:
        list(workers.map(fetch, CABS.items()))
    entries = {'manifest.json': manifest}
    for row in files:
        data = (cache / (row['cab'] + '.extracted') / row['id']).read_bytes()
        checked(data, row['sha256'])
        if len(data) != row['size']:
            raise RuntimeError('Unexpected debugger file size')
        entries[row['path']] = data
    archive = out / 'kf-kd-bundle.zip'
    with zipfile.ZipFile(archive, 'w') as package:
        for name, data in sorted(entries.items()):
            # Fixed controller-bundle metadata, independent of download time and umask.
            info = zipfile.ZipInfo(name, (2026, 10, 5, 1, 59, 36))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            package.writestr(info, data, compresslevel=3)
    checked(archive.read_bytes(), ZIP_SHA)
    shutil.copyfile(here / 'analyze-dump.ps1', out / 'analyze-dump.ps1')
    print(f'{ZIP_SHA}  {archive}')


if __name__ == '__main__':
    main()
