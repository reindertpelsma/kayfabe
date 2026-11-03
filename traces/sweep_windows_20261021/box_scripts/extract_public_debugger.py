"""Rental-only CRC-checked extraction of the same official Microsoft ZIP64 bundle.

Ubuntu's old 7z rejected the outer headers; that failure remains in its receipt.
This does not change the download or the Windows Authenticode execution guard.
"""
from pathlib import Path, PurePosixPath
from zipfile import ZipFile
import hashlib, json, shutil

root = Path('/root/sweep2/windbg_pkg')
bundle = root / 'windbg.msixbundle'
assert bundle.stat().st_size == 1203512387
assert hashlib.sha256(bundle.read_bytes()).hexdigest() == '4662ae17bc0dc77db987b89cccea32c30058baa06268f861d09e0ab4e4f45c20'
package = root / 'windbg_win-x64.msix'
with ZipFile(bundle) as archive:
    entry = archive.getinfo('windbg_win-x64.msix')
    assert entry.file_size == 404832537
    with archive.open(entry) as src, package.open('wb') as dst:
        shutil.copyfileobj(src, dst, 1024 * 1024)
receipt = []
with ZipFile(package) as archive:
    entries = [e for e in archive.infolist() if e.filename.startswith('amd64/') and not e.is_dir()]
    assert 0 < len(entries) <= 2000
    assert sum(e.file_size for e in entries) <= 1024 * 1024 * 1024
    for entry in entries:
        relative = PurePosixPath(entry.filename)
        assert not relative.is_absolute() and '..' not in relative.parts
        target = root / 'x64' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        with archive.open(entry) as src, target.open('wb') as dst:
            shutil.copyfileobj(src, dst, 1024 * 1024)
        receipt.append({'path': entry.filename, 'bytes': entry.file_size, 'sha256': hashlib.sha256(target.read_bytes()).hexdigest()})
assert (root / 'x64/amd64/cdb.exe').is_file()
Path('/root/prov/public_debugger_extract.json').write_text(json.dumps({'bundle_sha256': hashlib.sha256(bundle.read_bytes()).hexdigest(), 'extractor': 'Python zipfile, CRC verified while reading every extracted entry', 'files': receipt}, indent=2) + '\n')
Path('/root/prov/public_debugger_extract.ready').write_text('PUBLIC_DEBUGGER_EXTRACT_READY (execution still requires valid Microsoft Authenticode)\n')
print('PUBLIC_DEBUGGER_EXTRACT_READY', len(receipt))
