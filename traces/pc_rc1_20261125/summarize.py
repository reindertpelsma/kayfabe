#!/usr/bin/env python3
"""Table of the Windows boots of traces/pc_rc1_20261125/ from out/<step>/ (run on the pc, text only, no host/user names).
usage: summarize.py <out-dir>   (prints JSON rows)"""
import json
import re
import sys
from pathlib import Path

out = Path(sys.argv[1])
rows = []
for d in sorted(out.iterdir()):
    if not d.is_dir() or not (d / "result.json").exists():
        continue
    r = json.loads((d / "result.json").read_text())
    poll = (d / "poll.txt").read_text().splitlines() if (d / "poll.txt").exists() else []
    ev = [json.loads(x) for x in (d / "qmp_events.jsonl").read_text().splitlines() if x.strip()]
    names = [e["event"] for e in ev if "event" in e]
    pan = [e for e in ev if e.get("event") == "GUEST_PANICKED"]
    shut = [e for e in ev if e.get("event") == "SHUTDOWN"]
    c153 = [int(m.group(1)) for ln in poll for m in [re.search(r" 153=(\d+)", ln)] if m]
    adapters = [re.search(r"ADAPTER (.*?code=\d+)", ln).group(1) for ln in poll if "ADAPTER" in ln]
    smi = [ln for ln in poll if "NVSMI" in ln]
    pnp = (d / "pnp_at_end.txt").read_text().strip().replace("\n", " ; ") if (d / "pnp_at_end.txt").exists() else ""
    rows.append({
        "step": d.name, "arm": r["arm"], "up_at_s": r["up_at_s"], "alive_at_end": r["alive_at_end"], "qemu_exit_at_s": r["qemu_exit_at_s"],
        "max_nvlddmkm_153": max(c153) if c153 else 0,
        "qmp_events": [n for n in names if n not in ("RTC_CHANGE",)],
        "bugcheck": ("0x%X P1=%s" % (pan[0]["data"]["info"]["arg1"], hex(pan[0]["data"]["info"]["arg2"]))) if pan else "none",
        "shutdown_reason": shut[0]["data"]["reason"] if shut else "none",
        "adapter_last": adapters[-1] if adapters else "?", "pnp_at_end": pnp[:400],
        "host_mmuWalkMap": r["host_mmuWalkMap_lines"], "host_no_memory_assert": r["host_no_memory_lines"],
        "log_counts": r.get("log_counts"),
    })
print(json.dumps(rows, indent=1))
