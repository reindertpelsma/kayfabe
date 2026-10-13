import io, subprocess, sys, tarfile
from pathlib import Path

revision = sys.argv[1]
assert len(revision) == 9 and all(c in '0123456789abcdef' for c in revision)
base = Path('/var/lib/kf-windows-20261005')
assert subprocess.run(['pgrep', 'qemu-system'], stdout=subprocess.DEVNULL).returncode == 1
roots = {
    'linux': base / ('codex-memory-access-linux-' + revision),
    'desktop': base / ('codex-linux-desktop-' + revision),
}
with tarfile.open(fileobj=sys.stdout.buffer, mode='w|') as archive:
    for label, root in roots.items():
        assert (root / 'EXIT').read_text().strip() == '0'
        for path in sorted(root.rglob('*')):
            if not path.is_file() or path.is_symlink():
                continue
            if path.name not in ('START', 'EXIT', 'binary.sha256', 'iommu-after.txt', 'remaining-qemu.log', 'guest-smoke.log', 'host-xid.log', 'launcher.log') and path.suffix not in ('.log', '.out', '.txt'):
                continue
            assert path.stat().st_size < 64 << 20, path
            archive.add(path, arcname=label + '/' + str(path.relative_to(root)), recursive=False)
