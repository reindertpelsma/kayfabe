#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Patch the pinned native rental installer before staging it over SSH.

The resulting script exceeds Vast's 16 KiB onstart limit: SCP it and run bash.
This changes lab provisioning only; exact driver checks remain mandatory.
"""
import argparse
import hashlib
from pathlib import Path

# windows-on-vast 93b65b3347601f35f4146602ff85950e389b409f/install.sh (ISC).
SOURCE_SHA256 = "ac7faa15a606116ea162b2df95af9a5f7a163f0436989cfda92f2b7ab3e65af0"
ANCHOR = "param([switch]$Register)\n$ErrorActionPreference = 'Stop'\n"
POLICY = r"""# Exclude Windows Update driver packages before scheduling the pinned install.
# https://learn.microsoft.com/en-us/windows/deployment/update/waas-configure-wufb
# An earlier update may already be in flight; verify the actual build in each oracle.
$wuPolicy = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate'
New-Item -Path $wuPolicy -Force | Out-Null
New-ItemProperty -Path $wuPolicy -Name ExcludeWUDriversInQualityUpdate -PropertyType DWord -Value 1 -Force | Out-Null
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.source.resolve() == args.output.resolve():
        parser.error("source and output must differ")
    if args.source.stat().st_size > 1024 * 1024:
        parser.error("source exceeds the pinned installer size bound")
    data = args.source.read_bytes()
    if hashlib.sha256(data).hexdigest() != SOURCE_SHA256:
        parser.error("source is not the pinned public installer")
    source = data.decode("utf-8")
    if source.count(ANCHOR) != 1:
        parser.error("bootstrap hook is not unique")
    output = source.replace(ANCHOR, ANCHOR + POLICY, 1).encode("utf-8")
    args.output.write_bytes(output)
    print(f"NATIVE_INSTALLER bytes={len(output)} sha256={hashlib.sha256(output).hexdigest()}")


if __name__ == "__main__":
    main()
