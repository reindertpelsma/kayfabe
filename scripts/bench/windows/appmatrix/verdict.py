#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""verdict.py -- decide PASS / FAIL / TIMEOUT / NOTRUN for one app from the FACTS the guest supervisor
collected (guest/kf_run_app.ps1) and the app's log, with the same vocabulary and predicate shape as the Linux
matrix (scripts/apps/run_apps.sh): PASS = rc 0 AND the app's own success string AND no `CHECK ... FAIL`,
and on Windows additionally a GPU-USE PROOF and no work on a non-NVIDIA adapter (WARP excluded).

Pure functions, no I/O: unit-tested in tests/test_verdict.py.
"""
import re

VERDICTS = ("PASS", "FAIL", "TIMEOUT", "NOTRUN", "BOOT_FAIL")
# the Linux note extractor, plus the Windows failure words
NOTE_RX = re.compile(r"error|fail|illegal|cannot|unable|Xid|timeout|abort|segmentation|core dumped|no CUDA|not found|No such|exception|"
                     r"device removed|DXGI_ERROR|0x887A|Code 43|is not recognized|STILL_RUNNING|EXITED_EARLY|START_FAILED", re.I)
NOTE_SKIP_RX = re.compile(r"^=== |errors: 0|CHECK .* ok|Tests skipped: 0")
NOTRUN_RX = re.compile(r"is not recognized as the name of a cmdlet|The system cannot find (the )?(file|path)|Cannot find path|"
                       r"No such file or directory|KFERR|NOTFOUND|START_FAILED")
ENGINE_MIN_UTIL = 0.5          # percent: the GPU Engine counters are integers 0..100 per engine; below this is noise
OTHER_ADAPTER_MAX = 1.0        # percent on a non-NVIDIA LUID before it counts as "software adapter used"


def parse_proof(p):
    """'out:<regex>' | 'pdh:<engine regex>' | 'smi' -> (kind, arg)"""
    if p == "smi":
        return "smi", ""
    kind, _, arg = p.partition(":")
    if kind not in ("out", "pdh"):
        raise ValueError(f"unknown proof kind {p!r}")
    return kind, arg


def eval_proof(proofs, log, facts):
    """Return (ok, [descriptions of the proofs that held], [descriptions of those that did not])."""
    held, missed = [], []
    pdh = (facts.get("pdh") or {}).get("nv") or {}
    smi = facts.get("smi") or {}
    for p in proofs:
        kind, arg = parse_proof(p)
        if kind == "out":
            ok = re.search(arg, log, re.M) is not None
            (held if ok else missed).append("out")
        elif kind == "pdh":
            hit = sorted(e for e, v in pdh.items() if re.search(arg, e) and v and float(v) >= ENGINE_MIN_UTIL)
            (held if hit else missed).append(f"pdh:{'+'.join(hit) if hit else arg}")
        else:
            base = smi.get("mem_used_base_mb")
            mem = (smi.get("mem_used_max_mb") or 0) - (base or 0) if base is not None else 0
            ok = (smi.get("util_max") or 0) >= 1 or mem >= 64
            (held if ok else missed).append("smi")
    return (bool(held) or not proofs), held, missed


def first_note(log):
    for line in log.splitlines():
        if NOTE_RX.search(line) and not NOTE_SKIP_RX.search(line):
            return re.sub(r"\s+", " ", line).replace("|", "/").strip()[:160]
    return ""


def tdr_count(facts):
    ev = facts.get("events") or {}
    return int(ev.get("nvlddmkm_153", 0)) + int(ev.get("display_4101", 0))


def decide(app, facts, log):
    """app: an apps.json entry. facts: the supervisor's result JSON (or a synthesised one for a dead supervisor).
    log: the app's merged output. Returns a dict with verdict, rc, secs, quiet, note, proof, tdr, wer."""
    rc = facts.get("rc")
    out = dict(verdict="FAIL", rc="-" if rc is None else rc, secs=facts.get("secs", 0), quiet=facts.get("quiet", "-"), note="", proof="-",
               tdr=tdr_count(facts), wer=int((facts.get("events") or {}).get("wer_1001", 0)),
               rebooted=bool(facts.get("rebooted")))
    rx_ok = re.search(app["rx"], log, re.M) is not None
    fail_hit = bool(app.get("fail_rx")) and re.search(app["fail_rx"], log, re.M) is not None
    note = first_note(log)
    if facts.get("crash"):                                  # the supervisor died with the guest (bugcheck/reboot) or never answered
        out.update(verdict="FAIL", note=facts.get("crash"), rc=rc if rc is not None else "-")
        return out
    if facts.get("notrun"):
        out.update(verdict="NOTRUN", note=facts["notrun"])
        return out
    if facts.get("timed_out") or rc in (124, 137):
        out.update(verdict="TIMEOUT", note=note or "timeout")
        return out
    if NOTRUN_RX.search(log) and not rx_ok:
        out.update(verdict="NOTRUN", note=note or "missing precondition")
        return out
    ok, held, missed = eval_proof(app.get("proof") or [], log, facts)
    out["proof"] = ",".join(held) if held else ("none:" + ",".join(missed)[:60] if missed else "-")
    other = (facts.get("pdh") or {}).get("other") or {}
    sw = sorted(e for e, v in other.items() if v and float(v) > OTHER_ADAPTER_MAX)
    if rc == 0 and rx_ok and not fail_hit and ok and not (app.get("nvidia_only", True) and sw):
        out.update(verdict="PASS", note=("warn:" + note) if note else "")
        return out
    why = []
    if rc != 0:
        why.append(f"rc={rc}")
    if not rx_ok:
        why.append("success-string-missing")
    if fail_hit:
        why.append("fail-pattern")
    if not ok:
        why.append("no-gpu-proof")
    if sw and app.get("nvidia_only", True):
        why.append("non-NVIDIA-adapter:" + "+".join(sw))
    out.update(verdict="FAIL", note=(",".join(why) + (" | " + note if note else ""))[:200])
    return out


def digest_lines(log):
    return [l.strip() for l in log.splitlines() if re.match(r"^(OUTSHA|DIGEST) ", l)]


def appres_line(side, app_id, d, boot="-", gsp_cycles="-", extra=""):
    """The Linux `APPRES` line plus the Windows columns (guest_tdr, wer, proof, tier)."""
    note = re.sub(r"(\w+)=", r"\1:", (d.get("note") or "-").replace("\n", " "))   # summarize.py splits a line at ' key='; a note must not contain one
    return (f"APPRES side={side} app={app_id} verdict={d['verdict']} rc={d['rc']} secs={d['secs']} quiet={d['quiet']} note={note} "
            f"boot={boot} guest_tdr={d.get('tdr', 0)} wer={d.get('wer', 0)} gsp_cycles={gsp_cycles} proof={d.get('proof', '-')}{(' ' + extra) if extra else ''}")
