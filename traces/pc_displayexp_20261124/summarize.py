#!/usr/bin/env python3
"""Table of the display experiment from out/<step>/: one row per boot. Host dmesg lines are attributed to a boot by the host clock of the
QMP recorder (first connect .. socket close), converted to the dmesg monotonic stamp with the current uptime offset.
usage: summarize.py <out-dir> <host dmesg file saved on the pc>"""
import json
import re
import sys
from pathlib import Path

out = Path(sys.argv[1])
dm = Path(sys.argv[2]).read_text(errors="replace").splitlines()
off = float(sys.argv[3])   # time.time() - uptime at the moment the dmesg file was taken
rows = []
for d in sorted(out.iterdir(), key=lambda p: (p.name[0] != "A", p.name)):
    if not d.is_dir() or not (d / "result.json").exists():
        continue
    r = json.loads((d / "result.json").read_text())
    ev = [json.loads(x) for x in (d / "qmp_events.jsonl").read_text().splitlines() if x.strip()]
    ts = [e.get("t") or e.get("t_host") for e in ev if (e.get("t") or e.get("t_host"))]
    lo, hi = min(ts) - off - 3, max(ts) - off + 3
    win = [ln for ln in dm if re.match(r"\[\s*\d+\.\d+\]", ln) and lo <= float(re.match(r"\[\s*(\d+\.\d+)\]", ln).group(1)) <= hi]
    keep = [re.sub(r"GPU-[0-9a-fA-F-]{20,}", "GPU-<uuid>", ln) for ln in win if re.search(r"NVRM|Xid", ln)]
    (d / "host_dmesg_window.txt").write_text("\n".join(keep) + "\n")
    nomem = [ln for ln in win if "NV_ERR_NO_MEMORY" in ln]
    mmu = [ln for ln in win if "mmuWalkMap" in ln]
    poll = (d / "poll.txt").read_text().splitlines()
    c153 = [int(m.group(1)) for ln in poll for m in [re.search(r" 153=(\d+)", ln)] if m]
    # the first boots counted every event 153; the always-present Kernel-Boot 153 ("Virtualization-based security ... disabled") is
    # then subtracted when the last poll lists it. Later boots count nvlddmkm 153 only (the poll says so: a "153KernelBoot=" field).
    if c153 and "153KernelBoot=" not in " ".join(poll):
        kb = 1 if "Kernel-Boot" in (d / "events_last_successful_poll.txt").read_text() else 0
        c153 = [c - kb for c in c153]
    pan = [e for e in ev if e.get("event") == "GUEST_PANICKED"]
    shut = [e for e in ev if e.get("event") == "SHUTDOWN"]
    pnp = [ln for ln in poll if "ADAPTER" in ln]
    code = re.search(r"ADAPTER [^;]*?code=(\d+)", pnp[-1]).group(1) if pnp else "?"
    t_pan = (pan[0]["t_host"] - min(t for t in ts)) if pan else None
    rows.append({
        "step": d.name, "up_at_s": r["up_at_s"], "alive_at_end": r["alive_at_end"], "qemu_exit_at_s": r["qemu_exit_at_s"],
        "max_153": max(c153) if c153 else 0,
        "bugcheck": ("0x%X (%s,%s,%s,%s)" % (pan[0]["data"]["info"]["arg1"], hex(pan[0]["data"]["info"]["arg2"]),
                     hex(pan[0]["data"]["info"]["arg3"]), hex(pan[0]["data"]["info"]["arg4"]), hex(pan[0]["data"]["info"]["arg5"]))) if pan else "none",
        "shutdown_reason": shut[0]["data"]["reason"] if shut else "none",
        "adapter_code_last": code, "host_no_memory_lines": len(nomem), "host_mmuWalkMap_lines": len(mmu),
        "panic_after_recorder_s": round(t_pan, 1) if t_pan else None,
    })
print(json.dumps(rows, indent=1))
