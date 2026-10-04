#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Stage a trusted recorder bundle during vast-windows' final SSH rehearsal hold.

Does not start the driver, install NVIDIA, reboot, stop QEMU, release the hold,
write the host disk or arm the flasher. Test signing applies to the next boot.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import zipfile

HERE = Path(__file__).resolve().parent
DESTINATION = r'C:\ProgramData\KayfabeGsp'


def sha256(path):
    with path.open('rb') as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
        return digest.hexdigest()


def bundle(output):
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=HERE, text=True).strip()
    tracked = subprocess.check_output(['git', 'ls-files', '-z', '.'], cwd=HERE).decode().split('\0')
    files = [HERE / name for name in tracked if name]
    files += [HERE / 'build' / name for name in ('gsptrace.sys', 'gsptrace.exe', 'windows_api_test.exe')]
    files += sorted((HERE / 'build' / 'signing').glob('*'))
    if not (HERE / 'build/signing/signtool.exe').is_file():
        raise RuntimeError('Run build-linux.sh first; signing tools are required')
    for path in files:
        if not path.is_file() or path.is_symlink():
            raise RuntimeError(f'Not a regular build/source file: {path}')
    manifest = dict(schema='kayfabe-gsp-stage/1', source_revision=revision,
                    files=[dict(path=str(p.relative_to(HERE)).replace('\\', '/'), sha256=sha256(p), bytes=p.stat().st_size) for p in files])
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for path in files:
            archive.write(path, str(path.relative_to(HERE)))
        archive.writestr('stage-manifest.json', json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(dict(bundle=str(output), sha256=sha256(output), source_revision=revision, files=len(files))))


def stage(args):
    if sha256(args.bundle) != args.sha256.lower():
        raise RuntimeError('Recorder bundle checksum mismatch')
    hold = args.work / 'login-test.json'
    if not hold.is_file() or (args.work / 'login-test-complete').exists():
        raise RuntimeError('The final login rehearsal hold must be active; no guest changes made')
    spec = importlib.util.spec_from_file_location('vast_windows_prepare', args.prepare_helper)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    sock = args.work / 'qga.sock'

    def powershell(command):
        rc, out, error = helper.guest_exec(sock, ['-Command', command])
        if rc:
            raise RuntimeError(f'Guest PowerShell exited {rc}: {error}\n{out}')
        return out.strip()

    powershell(r'''$ErrorActionPreference='Stop'
if (-not (Test-Path C:\ProgramData\VastWindows\defer-native-gpu.flag)) { throw 'Native NVIDIA installation must be deferred before recorder staging' }
$null=Get-ScheduledTask -TaskName VastWindowsNativeGpu
New-Item -ItemType Directory -Path C:\ProgramData\KayfabeGsp -Force | Out-Null
& icacls.exe C:\ProgramData\KayfabeGsp /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
if ($LASTEXITCODE) { throw 'Could not protect recorder directory' }
''')
    destination = DESTINATION + r'\stage.zip'
    handle = helper.rpc(sock, 'guest-file-open', {'path': destination, 'mode': 'wb'})
    try:
        with args.bundle.open('rb') as stream:
            while block := stream.read(65536):
                result = helper.rpc(sock, 'guest-file-write', {'handle': handle, 'buf-b64': base64.b64encode(block).decode('ascii')})
                if result.get('count') != len(block):
                    raise RuntimeError('Short guest-file-write; staging archive is incomplete')
        helper.rpc(sock, 'guest-file-flush', {'handle': handle})
    finally:
        helper.rpc(sock, 'guest-file-close', {'handle': handle})
    check = r'''$ErrorActionPreference='Stop'
$root='C:\ProgramData\KayfabeGsp'
if ((Get-FileHash "$root\stage.zip" -Algorithm SHA256).Hash -ne '__SHA256__') { throw 'Windows staging archive SHA256 mismatch' }
Expand-Archive -LiteralPath "$root\stage.zip" -DestinationPath $root -Force
$manifest=Get-Content -Raw "$root\stage-manifest.json" | ConvertFrom-Json
if ($manifest.schema -ne 'kayfabe-gsp-stage/1') { throw 'Unknown manifest schema' }
foreach ($entry in $manifest.files) {
    $path=[IO.Path]::GetFullPath((Join-Path $root $entry.path))
    if (-not $path.StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Path escapes recorder directory' }
    if ((Get-Item -LiteralPath $path).Length -ne $entry.bytes -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.sha256) { throw "Staging mismatch: $($entry.path)" }
}
& "$root\build\signing\signtool.exe" /? | Out-Null
if ($LASTEXITCODE) { throw 'Bundled Microsoft signtool cannot run on this Windows installation' }
$collectorStatus=& $env:ComSpec /d /c ($root+'\build\gsptrace.exe --status 2>&1') | Out-String
$collectorExit=$LASTEXITCODE
if ($collectorExit -ne 1 -or $collectorStatus -notmatch 'Open driver: Win32') { throw "Unexpected collector pre-load result: $collectorStatus" }
& "$root\install.ps1" -Action EnableTestSigning | Out-Null
$bcd=& bcdedit.exe /enum '{current}' | Out-String
if ($LASTEXITCODE -or $bcd -notmatch '(?im)^\s*testsigning\s+Yes\s*$') { throw "testsigning not recorded in BCD: $bcd" }
$deviceGuard=$null;$deviceGuardError=$null;$secureBoot=$null
try { $deviceGuard=Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard | Select-Object VirtualizationBasedSecurityStatus,SecurityServicesConfigured,SecurityServicesRunning,CodeIntegrityPolicyEnforcementStatus } catch { $deviceGuardError=$_.Exception.Message }
try { $secureBoot=Confirm-SecureBootUEFI } catch { }
$stage=[ordered]@{schema='kayfabe-gsp-stage-result/1';secure_boot=$secureBoot;device_guard=$deviceGuard;device_guard_query_error=$deviceGuardError;source_revision=$manifest.source_revision;verified_files=@($manifest.files).Count;signtool_ran=$true;collector_ran=$true;testsigning_next_boot=$true;driver_loaded=$false;native_install_deferred=(Test-Path C:\ProgramData\VastWindows\defer-native-gpu.flag);reboot_performed=$false;staged_utc=(Get-Date).ToUniversalTime().ToString('o')}
$stage | ConvertTo-Json -Depth 5 | Set-Content "$root\stage-result.json" -Encoding UTF8
$stage | ConvertTo-Json -Depth 5 -Compress
'''.replace('__SHA256__', args.sha256.lower())
    result = json.loads(powershell(check))
    (args.work / 'gsp-stage-result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    pack = commands.add_parser('bundle', help='Run on the trusted build controller')
    pack.add_argument('output', type=Path)
    upload = commands.add_parser('stage', help='Run on Linux rental during the final coldboot hold')
    upload.add_argument('bundle', type=Path)
    upload.add_argument('--sha256', required=True)
    upload.add_argument('--work', type=Path, default=Path('/var/lib/vast-windows'))
    upload.add_argument('--prepare-helper', type=Path, default=Path('/root/vast-windows/prepare/prepare.py'))
    args = parser.parse_args()
    try:
        if args.command == 'bundle':
            bundle(args.output)
        else:
            if len(args.sha256) != 64 or any(c not in '0123456789abcdefABCDEF' for c in args.sha256):
                raise ValueError('Expected SHA256 must contain 64 hexadecimal characters')
            stage(args)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f'REFUSED: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
