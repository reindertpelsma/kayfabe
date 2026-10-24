"""Back up native text receipts, then send the existing fixed ACK; no publication gate."""

from pathlib import Path
import datetime as dt
import hashlib
import json
import shutil
import subprocess
import sys

D = Path(__file__).resolve().parent
T = Path("/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/ROUND_B1_1830")
subprocess.run([sys.executable, str(D / "export_meta.py")], check=True)
source = T / "lease_metadata"
dest = T / "native_baseline"
assert not dest.exists()
shutil.copytree(source, dest)
sha = hashlib.sha256((dest / "export_receipt.json").read_bytes()).hexdigest()
script = (D / "ack_native_template.py").read_text().replace("__LOCAL_EXPORT_RECEIPT_SHA256__", sha)
r = subprocess.run(
    ["ssh", "-F", str(D / "ssh.conf"), (D / "alias").read_text().strip(), "python3 -I -"],
    input=script,
    capture_output=True,
    text=True,
    check=True,
    timeout=40,
)
row = json.loads(r.stdout)
row.update(
    utc_local=dt.datetime.now(dt.timezone.utc).isoformat(),
    host_verification="UNVERIFIED-HOST",
    experiment_mode="OBSERVATION",
    attestation=False,
)
(T / "native_backup_ack.json").write_text(json.dumps(row, indent=2) + "\n")
print(json.dumps(row))
