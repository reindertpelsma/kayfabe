#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box-side driver for a guest that propimage_check.py holds up (`--hold-until-file`): run a PowerShell script or a list of desktop actions in the
guest's interactive session (normal integrity, user kfimage), wait for its result.json and pull the small text/PNG files back.
STATUS: LIVE, 2026-11-22 (first used by the Adobe part-2 run, traces/adobe_vast_part2_20261122/). Stdlib only. Run it from a checkout:

  drive.py --work DIR ps 'one-line powershell'                       DIR = the harness' --work directory (state.json, id_kf)
  drive.py --work DIR script FILE.ps1 --name N [--var K=V ...] [--timeout S] [--out DIR]
  drive.py --work DIR ui --name N [--out DIR] ACTION_LINES...          actions: shot NAME | click X Y | dclick | rclick | drag X1 Y1 X2 Y2 | key KEYS | type TEXT | wait S | focus PROC | titles
  drive.py --work DIR row ROWNAME [--out DIR]                          one harness row (apps_b table, e.g. aeadv3d) in the held guest, same code as the stage
  drive.py --work DIR prep                                             elevated: WER LocalDumps (crash dumps stay in the guest, C:\\kfapps\\dumps)

The guest script gets $D (its run directory), $APPNAME, every --var as a single-quoted string, then lib.ps1 (apps_b), uitext_lib.ps1 and ui_lib.ps1; it ends with Finish $R."""
import argparse
import importlib.util
import json
import re
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
import os
WIN = Path(os.environ.get("KF_WIN", str(HERE.parent)))
sys.path.insert(0, str(WIN))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def guest_of(work):
    pc = load("propimage_check", WIN / "propimage_check.py")
    state = json.loads((Path(work) / "state.json").read_text())
    key = Path(work) / "id_kf"
    if not key.is_file():
        raise SystemExit("no id_kf in %s: the harness is not holding a guest" % work)
    return pc.Guest(state, str(key), state["ids"]["net"]["ssh_port"])


def q(value):
    return "'" + str(value).replace("'", "''") + "'"


def render(name, run_id, body_file, variables, here=HERE):
    lib = (WIN / "apps_b/lib.ps1").read_text()
    head = "$D = 'C:\\kfapps\\p2\\%s\\%s'\n$APPNAME = %s\n" % (run_id, name, q(name))
    for k, v in variables.items():
        if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", k):
            raise SystemExit("bad variable name " + k)
        head += "$%s = %s\n" % (k, q(v))
    body = Path(body_file).read_text().replace(". 'C:\\kfhw\\lib.ps1'\n", "")
    return head + lib + "\n" + (here / "uitext_lib.ps1").read_text() + "\n" + (here / "ui_lib.ps1").read_text() + "\n" + body


def run_guest_script(g, name, text, run_id, timeout, out):
    ab = load("propimage_apps_b", WIN / "propimage_apps_b.py")
    task = "p2_" + run_id[:12]
    remote = "C:/kfapps/p2/%s/%s" % (run_id, name)
    t0 = time.monotonic()
    p = ab._interactive(g, task, text)
    if p.returncode:
        raise SystemExit("task start failed: " + (p.stdout + p.stderr)[:300])
    while time.monotonic() - t0 < timeout:
        r = g.ps("if(Test-Path '%s/result.json'){'ready'}" % remote)
        if r.returncode == 0 and r.stdout.strip() == "ready":
            break
        if r.returncode:
            print("transport trouble:", r.stderr[:200])
        time.sleep(5)
    else:
        print("TIMEOUT after %ds; pulling what exists" % timeout)
    out = Path(out) / ("%s-%s" % (name, run_id[:8]))
    out.mkdir(parents=True, exist_ok=True)
    ls = g.ps("Get-ChildItem '%s' -ErrorAction SilentlyContinue | ForEach-Object { '{0}`t{1}' -f $_.Name, $_.Length }" % remote.replace("/", "\\"))
    for line in ls.stdout.splitlines():
        fn, _, size = line.strip().partition("\t")
        if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}", fn) and size.isdigit() and int(size) <= 4 * 1024 * 1024 and fn.rsplit(".", 1)[-1] in ("png", "txt", "json", "log"):
            g.scp(remote + "/" + fn, str(out / fn), False)
    g.ps("Stop-ScheduledTask -TaskName kf_%s -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName kf_%s -Confirm:$false -ErrorAction SilentlyContinue" % (task, task))
    res = out / "result.json"
    print("OUT", out)
    if res.is_file():
        print(res.read_text(encoding="utf-8-sig")[:6000])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--work", required=True)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("ps"); s.add_argument("command"); s.add_argument("--timeout", type=int, default=120)
    sub.add_parser("prep")
    s = sub.add_parser("row"); s.add_argument("rowname"); s.add_argument("--out", default="/workspace/p2/out")
    s = sub.add_parser("script"); s.add_argument("file"); s.add_argument("--name", required=True); s.add_argument("--var", action="append", default=[]); s.add_argument("--timeout", type=int, default=900); s.add_argument("--out", default="/workspace/p2/out")
    s = sub.add_parser("ui"); s.add_argument("--name", default="ui"); s.add_argument("--timeout", type=int, default=300); s.add_argument("--out", default="/workspace/p2/out"); s.add_argument("actions", nargs="+")
    a = ap.parse_args()
    g = guest_of(a.work)
    if a.cmd == "ps":
        r = g.ps(a.command, timeout=a.timeout)
        print(r.stdout, r.stderr[:500], "rc", r.returncode)
    elif a.cmd == "row":
        ab = load("propimage_apps_b", WIN / "propimage_apps_b.py")
        Path(a.out).mkdir(parents=True, exist_ok=True)
        print(json.dumps(ab.run_check(a.rowname, a.out, g), indent=1)[:4000])
    elif a.cmd == "prep":
        ab = load("propimage_apps_b", WIN / "propimage_apps_b.py")
        r = g.ps(ab.LOCALDUMPS_SETUP)
        print("prep rc", r.returncode, r.stderr[:200])
    else:
        run_id = uuid.uuid4().hex
        if a.cmd == "script":
            variables = dict(v.split("=", 1) for v in a.var)
            body = a.file
        else:
            variables = {"ACTIONS": "\n".join(a.actions)}
            body = HERE / "ui_actions.ps1"
        run_guest_script(g, a.name, render(a.name, run_id, body, variables), run_id, a.timeout, a.out)


if __name__ == "__main__":
    main()
