#!/usr/bin/env python3
# pull_guest.py WORKDIR GUEST_DIR OUT_DIR : list a guest directory (C:\kfapps\...) and scp every small png/txt/json/log file out, printing scp errors (drive.py swallows them).
import importlib.util, json, os, re, subprocess, sys
from pathlib import Path
WIN = Path(os.environ.get("KF_WIN", "/root/kayfabe/scripts/bench/windows"))
spec = importlib.util.spec_from_file_location("propimage_check", WIN / "propimage_check.py"); pc = importlib.util.module_from_spec(spec); sys.modules["propimage_check"] = pc; spec.loader.exec_module(pc)
work, gdir, out = sys.argv[1:4]
st = json.loads((Path(work) / "state.json").read_text())
g = pc.Guest(st, str(Path(work) / "id_kf"), st["ids"]["net"]["ssh_port"])
Path(out).mkdir(parents=True, exist_ok=True)
ls = g.ps("Get-ChildItem '%s' -Recurse -File | ForEach-Object { '{0}|{1}' -f $_.FullName, $_.Length }" % gdir, timeout=200)
print("ls rc", ls.returncode, ls.stderr[:200])
for line in ls.stdout.splitlines():
    full, _, size = line.strip().rpartition("|")
    if not full or not size.isdigit():
        continue
    name = full.replace("\\", "/").split("/")[-1]
    if int(size) <= 4 << 20 and name.rsplit(".", 1)[-1] in ("png", "txt", "json", "log") and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}", name):
        parts = full.replace("\\", "/").split("/")
        r = g.scp(full.replace("\\", "/"), str(Path(out) / (parts[-2] + "__" + name)), False)
        print(name, size, "rc", r.returncode, (r.stderr or "")[:120])
