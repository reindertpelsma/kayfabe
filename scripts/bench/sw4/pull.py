from pathlib import Path
import datetime as dt
import hashlib
import json
import subprocess
import tarfile

D = Path(__file__).resolve().parent
E = Path((D / "current").read_text())
conf = str(E / "ssh.conf")
subprocess.run(
    ["scp", "-F", conf, str(D / "collect_capture.py"), "sw4-own:/root/sw4/collect_capture.py"],
    capture_output=True,
    check=True,
    timeout=60,
)
r = subprocess.run(
    ["ssh", "-F", conf, "sw4-own", "python3 /root/sw4/collect_capture.py"],
    capture_output=True,
    text=True,
    check=True,
    timeout=60,
)
receipt = json.loads(r.stdout)
assert 0 < receipt["bytes"] <= 500_000_000
subprocess.run(
    ["scp", "-F", conf, "sw4-own:/root/sw4/capture.tgz", str(E / "capture.tgz")],
    capture_output=True,
    check=True,
    timeout=120,
)
with (E / "capture.tgz").open("rb") as stream:
    actual = hashlib.file_digest(stream, "sha256").hexdigest()
assert actual == receipt["sha256"]
output = E / ("captured_" + dt.datetime.now(dt.timezone.utc).strftime("%H%M%S"))
output.mkdir()
with tarfile.open(E / "capture.tgz", "r:gz") as archive:
    members = archive.getmembers()
    assert len(members) == receipt["files"] and len(members) <= 2000
    for member in members:
        path = Path(member.name)
        assert member.isfile() and not path.is_absolute() and ".." not in path.parts
        assert path.parts[0] in ("evidence", "bench")
        destination = output / path
        destination.parent.mkdir(exist_ok=True, parents=True)
        with archive.extractfile(member) as src, destination.open("xb") as dst:
            dst.write(src.read())
receipt.update(
    local_sha256=actual,
    verified_utc=dt.datetime.now(dt.timezone.utc).isoformat(),
    extracted_directory=str(output),
)
(E / "capture_receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt, indent=2))
