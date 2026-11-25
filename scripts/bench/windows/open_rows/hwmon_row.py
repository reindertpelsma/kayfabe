# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Runner and judge of the LibreHardwareMonitor row (STATUS: DESIGN-ONLY, 2026-11-23; logic GPU-free, never run on a Windows guest or on native Windows).

LibreHardwareMonitor's library is the first real NVAPI consumer of the Windows rows: it asks the driver (nvapi64.dll) for the GPU's temperatures, clocks,
loads, memory sizes and fan. lhm_dump.ps1 loads the library from the pinned release zip and prints what it read as one JSON line; this module judges that
document in named checks (the style of gl_soak.py) and compares it with the guest's own `nvidia-smi` where that answers a number.

What PASS means: the library found NVIDIA hardware of the type it names for NVAPI GPUs, every sensor type the row requires (temperature, clock, load, memory
size) has a numeric value, the temperature and load values are physically plausible, and `nvidia-smi` (when it answers numbers) agrees within a tolerance.
A sensor that exists with no value is a finding (`sensor_value_missing`), not a skipped check: NVAPI answering "no data" is what a hardware monitor under
a virtualised driver shows first.

GPU use: a sensor reader does no GPU work, so the assertion is "the answer came from the NVIDIA GPU and not from a fallback": the hardware record must be the
NVAPI type and carry an NVIDIA name. The guest self-report is all the Windows rows have (docs/design/V3_GPU_USE_ASSERTION.md section 2.2); the negative control
(`--negctl`) runs the same reader with its GPU group disabled, and the row is a good control only when this judge says FAIL for it.
"""
import json
import os
import re

from kfopen_common import HERE, OpenRowError, check_line, say

DUMP_PS1 = os.path.join(HERE, "lhm_dump.ps1")
POWERSHELL = "powershell.exe"
SMI_FIELDS = "name,temperature.gpu,memory.total"
NVIDIA_RX = re.compile(r"nvidia|geforce|rtx|quadro|tesla", re.I)


def parse_dump(text):
    """The one JSON line lhm_dump.ps1 prints -> dict, or raises OpenRowError (no JSON, not an object). Anything before the last line is ignored."""
    lines = [l for l in (text or "").strip().splitlines() if l.strip()]
    if not lines:
        raise OpenRowError("DUMP_UNREADABLE the reader printed nothing")
    try:
        doc = json.loads(lines[-1])
    except ValueError:
        raise OpenRowError("DUMP_UNREADABLE the last line of the reader is not JSON: %s" % lines[-1][:80])
    if not isinstance(doc, dict) or not isinstance(doc.get("hardware"), list):
        raise OpenRowError("DUMP_UNREADABLE the document has no hardware list")
    return doc


def walk(hardware):
    """Every hardware record, sub-hardware included, in order."""
    for h in hardware or []:
        if isinstance(h, dict):
            yield h
            for s in walk(h.get("sub")):
                yield s


def as_list(x):
    """PowerShell's ConvertTo-Json writes a one-element array as the element itself: normalise."""
    return x if isinstance(x, list) else ([] if x is None else [x])


def gpu_records(doc, hw_type):
    return [h for h in walk(as_list(doc.get("hardware"))) if h.get("type") == hw_type and NVIDIA_RX.search(str(h.get("name", "")))]


def numeric(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def sensors_of(records):
    return [s for h in records for s in as_list(h.get("sensors")) if isinstance(s, dict)]


def parse_smi(text):
    """`nvidia-smi --query-gpu=name,temperature.gpu,memory.total --format=csv,noheader,nounits` -> {name, temperature_c, memory_mib} (None for [N/A]), or None."""
    line = next((l for l in (text or "").splitlines() if l.strip()), "")
    cells = [c.strip() for c in line.split(",")]
    if len(cells) != 3:
        return None

    def num(c):
        return float(c) if re.fullmatch(r"\d+(\.\d+)?", c) else None
    return {"name": cells[0], "temperature_c": num(cells[1]), "memory_mib": num(cells[2])}


def judge(row, doc, smi=None):
    """-> (checks [(name, ok, why)], facts). `row` is the open_rows.json row, `doc` the parsed dump, `smi` parse_smi's dict or None."""
    gpus = gpu_records(doc, row["gpu_hardware_type"])
    sensors = sensors_of(gpus)
    types = sorted({s.get("type") for s in sensors})
    lo, hi = row["temperature_c"]
    llo, lhi = row["load_percent"]
    checks = [
        ("lhm_no_reader_errors", not as_list(doc.get("errors")), "; ".join(str(e)[:100] for e in as_list(doc.get("errors"))[:2])),
        ("lhm_nvidia_hardware", bool(gpus), "no %s hardware named like an NVIDIA GPU among %s" % (
            row["gpu_hardware_type"], sorted({"%s:%s" % (h.get("type"), str(h.get("name"))[:30]) for h in walk(as_list(doc.get("hardware")))})[:4])),
    ]
    for t in row["required_sensor_types"]:
        vals = [s["value"] for s in sensors if s.get("type") == t and numeric(s.get("value"))]
        present = [s for s in sensors if s.get("type") == t]
        why = ("no %s sensor at all" % t) if not present else "sensor_value_missing: %d %s sensor(s), none has a value" % (len(present), t)
        checks.append(("lhm_sensor_" + t.lower(), bool(vals), why))
    temps = [s["value"] for s in sensors if s.get("type") == "Temperature" and numeric(s.get("value"))]
    checks.append(("lhm_temperature_plausible", all(lo <= v <= hi for v in temps), "temperature outside %g..%g C: %s" % (lo, hi, [v for v in temps if not lo <= v <= hi][:3])))
    loads = [s["value"] for s in sensors if s.get("type") == "Load" and numeric(s.get("value"))]
    checks.append(("lhm_load_plausible", all(llo <= v <= lhi for v in loads), "load outside %g..%g percent: %s" % (llo, lhi, [v for v in loads if not llo <= v <= lhi][:3])))
    clocks = [s["value"] for s in sensors if s.get("type") == "Clock" and numeric(s.get("value"))]
    checks.append(("lhm_clock_positive", all(v > 0 for v in clocks), "a clock of 0 MHz or less: %s" % [v for v in clocks if v <= 0][:3]))
    mem = [s["value"] for s in sensors if s.get("type") == "SmallData" and "total" in str(s.get("name", "")).lower() and numeric(s.get("value"))]
    checks.append(("lhm_memory_total_positive", all(v > 0 for v in mem), "memory total of 0: %s" % mem[:3]))
    tol = row["smi_tolerance"]
    if smi and smi["temperature_c"] is not None and temps:
        d = abs(max(temps) - smi["temperature_c"])
        checks.append(("lhm_agrees_with_smi_temperature", d <= tol["temperature_c"], "LHM %g C against nvidia-smi %g C" % (max(temps), smi["temperature_c"])))
    if smi and smi["memory_mib"] and mem:
        frac = abs(max(mem) - smi["memory_mib"]) / smi["memory_mib"]
        checks.append(("lhm_agrees_with_smi_memory", frac <= tol["memory_fraction"], "LHM %g MB against nvidia-smi %g MiB" % (max(mem), smi["memory_mib"])))
    facts = {"gpu": gpus[0]["name"] if gpus else "", "lib": doc.get("lib_version", "?"), "hardware": len(list(walk(as_list(doc.get("hardware"))))),
             "sensors": len(sensors), "types": types, "gpu_enabled": doc.get("gpu_enabled", True)}
    return checks, facts


def dump_argv(lib_dir, negctl=False):
    return [POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", DUMP_PS1, "-LibDir", lib_dir, "-GpuEnabled", "0" if negctl else "1"]


def smi_argv():
    return ["nvidia-smi.exe", "--query-gpu=" + SMI_FIELDS, "--format=csv,noheader,nounits"]


def run_hwmon(row_id, row, runner, lib_dir, negctl=False):
    """0 PASS, 1 FAIL. With negctl the exit status is 0 only when the judge says FAIL for the GPU-disabled reader (CONTROL_OK); it never prints a PASS verdict
    and a reader that did not run is CONTROL_BROKEN."""
    say("START", "row=%s negctl=%d" % (row_id, int(negctl)))
    res = runner.run(dump_argv(lib_dir, negctl), timeout=row["timeout_s"])
    try:
        if res.rc != 0:
            raise OpenRowError("DUMP_FAILED the reader exited %s" % res.rc)
        doc = parse_dump(res.out)
    except OpenRowError as e:
        check_line("lhm_dump", False, str(e))
        if negctl:                                         # a reader that did not run is no control: the judge never saw a GPU-less document
            say("NEGCTL", "CONTROL_BROKEN row=%s judged=ERROR" % row_id)
        else:
            say("VERDICT", "FAIL row=%s failed=lhm_dump" % row_id)
        return 1
    smi_res = runner.run(smi_argv(), timeout=60)
    smi = parse_smi(smi_res.out) if smi_res.rc == 0 else None
    checks, facts = judge(row, doc, smi)
    for name, ok, why in checks:
        check_line(name, ok, why)
    say("HWMON", "gpu=%s lib=%s hardware=%d sensors=%d types=%s smi=%s" % (facts["gpu"].replace(" ", "_") or "none", facts["lib"], facts["hardware"], facts["sensors"],
                                                                     "+".join(facts["types"]) or "none", "answered" if smi else "n/a"))
    failed = [c[0] for c in checks if not c[1]]
    if negctl:
        control = "CONTROL_OK" if failed else "CONTROL_BROKEN"
        say("NEGCTL", "%s row=%s judged=%s" % (control, row_id, "FAIL" if failed else "PASS"))
        return 0 if failed else 1
    say("VERDICT", ("PASS row=%s sensors=%d classes=%d" % (row_id, facts["sensors"], len(facts["types"]))) if not failed else ("FAIL row=%s failed=%s" % (row_id, ",".join(failed))))
    return 0 if not failed else 1
