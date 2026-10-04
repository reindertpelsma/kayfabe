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


def bundle(output, build_directory=None):
    build = build_directory or HERE / 'build'
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=HERE, text=True).strip()
    tracked = subprocess.check_output(['git', 'ls-files', '-z', '.'], cwd=HERE).decode().split('\0')
    files = [(HERE / name, name) for name in tracked if name]
    required = ('gsptrace.sys', 'gsptrace.exe', 'windows_api_test.exe')
    files += [(build / name, f'build/{name}') for name in required]
    metadata = None
    if build_directory is not None and not (build / 'build-info.json').is_file():
        raise RuntimeError('External build directory requires build-info.json provenance')
    if (build / 'build-info.json').is_file():
        metadata = json.loads((build / 'build-info.json').read_text(encoding='utf-8-sig'))
        if metadata.get('completed') is not True or metadata.get('signing_performed') is not False:
            raise RuntimeError('Build metadata must describe a completed unsigned build')
        artifacts = {entry['name']: entry for entry in metadata['artifacts']}
        for name in required:
            entry = artifacts.get(name, {})
            if entry.get('sha256') != sha256(build / name) or entry.get('bytes') != (build / name).stat().st_size:
                raise RuntimeError(f'Build provenance mismatch: {name}')
    for name in ('build-info.json', 'build.log', 'driver-pe.json'):
        if (build / name).is_file():
            files.append((build / name, f'build/{name}'))
    signing = build / 'signing'
    if not (signing / 'signtool.exe').is_file():
        signing = HERE / 'build' / 'signing'
    files += [(path, f'build/signing/{path.name}') for path in sorted(signing.glob('*'))]
    if not (signing / 'signtool.exe').is_file():
        raise RuntimeError('Run build-linux.sh first; signing tools are required')
    for path, _ in files:
        if not path.is_file() or path.is_symlink():
            raise RuntimeError(f'Not a regular build/source file: {path}')
    manifest = dict(schema='kayfabe-gsp-stage/1', source_revision=revision,
                    build_source_revision=metadata.get('source_revision') if metadata else None,
                    files=[dict(path=name.replace('\\', '/'), sha256=sha256(path), bytes=path.stat().st_size) for path, name in files])
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for path, name in files:
            archive.write(path, name)
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

    def file_rpc(name, arguments):
        # Cold-boot Defender/servicing activity can delay QGA file operations.
        # Never retry a write whose acknowledgement was lost: its file offset
        # may already have advanced. A fresh staging invocation truncates and
        # verifies the complete archive instead.
        try:
            return helper.rpc(sock, name, arguments, timeout=60)
        except (OSError, EOFError, ValueError) as error:
            raise RuntimeError(f'{name} failed; upload was not retried: {error}') from error

    powershell(r'''$ErrorActionPreference='Stop'
if (-not (Test-Path C:\ProgramData\VastWindows\defer-native-gpu.flag)) { throw 'Native NVIDIA installation must be deferred before recorder staging' }
$null=Get-ScheduledTask -TaskName VastWindowsNativeGpu
New-Item -ItemType Directory -Path C:\ProgramData\KayfabeGsp -Force | Out-Null
& icacls.exe C:\ProgramData\KayfabeGsp /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
if ($LASTEXITCODE) { throw 'Could not protect recorder directory' }
''')
    destination = DESTINATION + r'\stage.zip'
    print('Recorder directory ready; uploading trusted archive', file=sys.stderr, flush=True)
    handle = file_rpc('guest-file-open', {'path': destination, 'mode': 'wb'})
    uploaded = 0
    try:
        with args.bundle.open('rb') as stream:
            while block := stream.read(65536):
                result = file_rpc('guest-file-write', {'handle': handle, 'buf-b64': base64.b64encode(block).decode('ascii')})
                if result.get('count') != len(block):
                    raise RuntimeError('Short guest-file-write; staging archive is incomplete')
                uploaded += len(block)
                if uploaded % (1024 * 1024) == 0:
                    print(f'Uploaded {uploaded} archive bytes', file=sys.stderr, flush=True)
        file_rpc('guest-file-flush', {'handle': handle})
    finally:
        file_rpc('guest-file-close', {'handle': handle})
    print(f'Uploaded {uploaded} bytes; verifying and enabling test signing for next boot', file=sys.stderr, flush=True)
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
    pack.add_argument('--build-directory', type=Path, help='Trusted CI build directory; validates included build-info.json hashes')
    upload = commands.add_parser('stage', help='Run on Linux rental during the final coldboot hold')
    upload.add_argument('bundle', type=Path)
    upload.add_argument('--sha256', required=True)
    upload.add_argument('--work', type=Path, default=Path('/var/lib/vast-windows'))
    upload.add_argument('--prepare-helper', type=Path, default=Path('/root/vast-windows/prepare/prepare.py'))
    args = parser.parse_args()
    try:
        if args.command == 'bundle':
            bundle(args.output, args.build_directory)
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
