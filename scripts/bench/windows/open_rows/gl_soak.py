# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Runner and judge of the GL dual-context soak row (STATUS: DESIGN-ONLY, 2026-11-17; logic GPU-free, never run on a guest).

kf_gldual.exe (kf_gldual.c) runs two OpenGL 4.6 contexts in one process for --seconds. This module runs it, then asks the guest's event log
for the signature of the Resolve crash (Application Error 1000 / WER 1001 naming nvoglv64.dll, and the TDR recovery events) since the start.
It reuses the glcrash harness' conventions (scripts/bench/windows/glcrash/kfgl.ps1, vfio_gl_user.ps1): `KFGL key value` output, an exit code
that is 0 iff the last line says OK, the APPCRASH query on Application event 1000, an interactive session (the matrix row is session=interactive).
"""
import json
import os
import re

from kfopen_common import HERE, OpenRowError, check_line, say, utc

EVENTS_PS1 = os.path.join(HERE, "open_events.ps1")
POWERSHELL = "powershell.exe"


def kfgl_facts(text):
    """The KFGL lines of a run: renderer, version, ticks, result."""
    facts = {"renderer": None, "version": None, "ticks": [], "result": None, "failed": []}
    for line in text.splitlines():
        m = re.match(r"^KFGL (\S+) ?(.*)$", line.strip())
        if not m:
            continue
        k, v = m.group(1), m.group(2)
        if k == "renderer":
            facts["renderer"] = v
        elif k == "version":
            facts["version"] = v
        elif k == "tick":
            facts["ticks"].append(v)
        elif k == "RESULT":
            facts["result"] = v
        elif k == "FAILED":
            facts["failed"].append(v)
    return facts


def parse_events(text):
    """open_events.ps1 output -> [{Id, Provider, Message}]. An empty string or `null` is no events; anything else must be JSON."""
    t = (text or "").strip()
    if not t or t == "null":
        return []
    try:
        doc = json.loads(t.splitlines()[-1])
    except ValueError:
        raise OpenRowError("EVENTS_UNREADABLE the event query printed no JSON")
    return [doc] if isinstance(doc, dict) else list(doc)


def classify_events(events, exe_name="kf_gldual.exe", signatures=("nvoglv64.dll",)):
    """{crash_in_driver, app_crash, tdr} as lists of one-line descriptions."""
    out = {"crash_in_driver": [], "app_crash": [], "tdr": []}
    for e in events:
        msg, prov, i = e.get("Message", ""), e.get("Provider", ""), e.get("Id")
        if i in (1000, 1001) and any(s.lower() in msg.lower() for s in signatures):
            out["crash_in_driver"].append("%s %s" % (i, msg[:140]))
        elif i in (1000, 1001) and exe_name.lower() in msg.lower():
            out["app_crash"].append("%s %s" % (i, msg[:140]))
        elif (prov == "nvlddmkm" and i == 153) or (prov == "Display" and i == 4101):
            out["tdr"].append("%s/%s" % (prov, i))
    return out


def judge_soak(spec, rc, text, secs, events, smoke=False):
    """Named checks of a soak run. `spec` is the row (seconds). A run shorter than asked is a failure: the Resolve crash came minutes in."""
    f = kfgl_facts(text)
    want = spec["seconds_smoke"] if smoke else spec["seconds"]
    ev = classify_events(events, signatures=tuple(s for s in spec.get("crash_signatures", ["nvoglv64.dll"]) if s.endswith(".dll")))
    checks = [
        ("soak_result_ok", rc == 0 and (f["result"] or "").startswith("OK"), "rc=%s result=%s %s" % (rc, f["result"], " ".join(f["failed"]))),
        ("soak_duration", secs >= want - 5, "ran %.0f s of %d s" % (secs, want)),
        ("gl_renderer_nvidia", bool(f["renderer"]) and "NVIDIA" in f["renderer"], "GL_RENDERER %s" % f["renderer"]),
        ("no_nvoglv_crash", not ev["crash_in_driver"], "; ".join(ev["crash_in_driver"][:2])),
        ("no_app_crash", not ev["app_crash"], "; ".join(ev["app_crash"][:2])),
        ("no_tdr", not ev["tdr"], "%d TDR events (%s)" % (len(ev["tdr"]), ev["tdr"][0] if ev["tdr"] else "")),
    ]
    return checks, f, ev


def query_events(runner, since_iso):
    r = runner.run([POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", EVENTS_PS1, "-SinceUtc", since_iso], timeout=180)
    if r.rc != 0:
        raise OpenRowError("EVENTS_FAILED event query rc=%s" % r.rc)
    return parse_events(r.out)


def run_soak(row_id, spec, runner, exe, smoke=False, share=False):
    """Returns the exit code of the row: 0 PASS, 1 FAIL."""
    seconds = spec["seconds_smoke"] if smoke else spec["seconds"]
    since = utc()
    argv = [exe, "--seconds", str(seconds)] + (["--share"] if share else [])
    say("START", "row=%s seconds=%d share=%s" % (row_id, seconds, share))
    res = runner.run(argv, timeout=seconds + 180)
    events, events_ok = [], True
    try:
        events = query_events(runner, since)
    except OpenRowError as e:                               # no event log means no crash check: that is a failure, never a pass
        check_line("event_query", False, str(e))
        events_ok = False
    checks, facts, ev = judge_soak(spec, res.rc, res.out, res.secs, events, smoke)
    for name, ok, why in checks:
        check_line(name, ok, why)
    say("SOAK", "renderer=%s ticks=%d result=%s" % (facts["renderer"], len(facts["ticks"]), facts["result"]))
    ok = all(c[1] for c in checks) and events_ok
    say("VERDICT", ("PASS" if ok else "FAIL") + " row=%s seconds=%d share=%d" % (row_id, seconds, int(bool(share))))
    return 0 if ok else 1
