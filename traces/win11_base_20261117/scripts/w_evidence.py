#!/usr/bin/env python3
"""Copy the text evidence of the win11-base build to PROP_DIR/evidence_out/ with the new image's identity values and checksums redacted.
Runs ON the pc. Text only (no PNG). Identity values, the seed, password, key, computer name, MachineGuid and EK hash are replaced by <field>.
"""
import json
import re
import shutil
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa

st = json.loads((D / "state.json").read_text())
vals = {}
i = st["ids"]
for k, v in (("smbios_uuid", i["smbios"]["uuid"]), ("smbios_serial", i["smbios"]["serial"]), ("baseboard_serial", i["smbios"]["baseboard_serial"]),
             ("chassis_serial", i["smbios"]["chassis_serial"]), ("mac", i["nics"][0]["mac"]), ("disk_serial", i["disks"][0]["serial"]),
             ("gpu_uuid", i["gpu_uuid"]), ("subnet", i["net"]["subnet"].rsplit(".0/", 1)[0]), ("ssh_port", str(i["net"]["ssh_port"])),
             ("seed", st["seed"]), ("password", st["guest"]["password"]), ("machine_guid", st["guest"]["machine_guid"] or "none-recorded"),
             ("ek_pub_hash", st["guest"]["ek_pub_hash"] or "none-recorded"), ("computer_name", (D / "run/computer_name.txt").read_text().strip()),
             ("ssh_public_key_body", st["guest"]["ssh_public_key"].split()[1])):
    vals[k] = v
PUB = set(json.dumps({}).split()) | {PINS for PINS in (ISO["win_sha"], ISO["virtio_sha"])}
PI = load_pi()
PUB |= set(PI.PINS.values())


def redact(t):
    for k, v in vals.items():
        t = t.replace(v, "<%s>" % k)
        t = t.replace(v.lower(), "<%s>" % k).replace(v.upper(), "<%s>" % k)
    return re.sub(r"\b[0-9a-f]{64}\b", lambda m: m.group(0) if m.group(0) in PUB else "<sha256-redacted>", t)


E = D / "evidence_out"
if E.exists():
    shutil.rmtree(E)
E.mkdir(parents=True)
E.chmod(0o700)
n = 0
cands = []
for sub in ("run", "evidence"):
    base = D / sub
    if base.is_dir():
        for src in sorted(base.rglob("*")):
            if src.is_file() and not src.is_symlink() and src.suffix.lower() not in (".png", ".qcow2", ".iso", ".fd", ".efi", ".sock", ".fifo", ".pid") \
                    and src.stat().st_size < 3_000_000 and src.name not in ("id_kf", "state.json") and not src.name.startswith("state.") and "swtpm_setup" not in src.name and "shots" not in src.parts and "nvx" not in src.parts:
                cands.append(src)
cands += [p for p in sorted(D.glob("run_*.out"))] + [D / "media/tools.files.sha256"] + sorted(D.glob("sealed/*/manifest.json"))
for src in cands:
    if src.is_file():
        rel = src.relative_to(D)
        dst = E / str(rel).replace("/", "__")
        try:
            dst.write_text(redact(src.read_text(errors="replace")))
            n += 1
        except OSError:
            pass
(E / "scripts").mkdir()
for f in sorted((D / "bin").glob("w_*.py")):
    shutil.copyfile(f, E / "scripts" / f.name)
for f in ("propimage.py", "propimage_check.py", "propimage_autounattend.xml.in", "gsp_on.ps1", "desktop_prep.ps1", "gate_enable.ps1", "kf3_flags.sample.env", "kf3_census.py"):
    shutil.copyfile(D / "bin" / f, E / "scripts" / ("repo_" + f))
(E / "README.txt").write_text("Win11 base build evidence (text only, identity values and checksums redacted as <field>).\n")
print("copied", n, "files + scripts to", E)
