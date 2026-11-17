from pathlib import Path
import hashlib
import json
import tarfile

capture = Path("/root/sw4/capture.tgz")
roots = [(Path("/root/sw4/evidence"), "evidence"), (Path("/workspace/bench"), "bench")]
allowed = {".log", ".txt", ".json", ".out", ".rc"}
files = []
for root, prefix in roots:
    for path in root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix in allowed:
            files.append((path, prefix + "/" + path.name))
    controls = root / "native-controls"
    if controls.is_dir():
        for path in controls.iterdir():
            if path.is_file() and not path.is_symlink() and path.suffix in allowed:
                files.append((path, prefix + "/native-controls/" + path.name))
assert len(files) <= 2000 and sum(p.stat().st_size for p, _ in files) <= 2_000_000_000
with tarfile.open(capture, "w:gz") as archive:
    for path, name in files:
        archive.add(path, arcname=name, recursive=False)
hasher = hashlib.sha256()
with capture.open("rb") as stream:
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        hasher.update(chunk)
digest = hasher.hexdigest()
print(json.dumps({"sha256": digest, "bytes": capture.stat().st_size, "files": len(files)}))
