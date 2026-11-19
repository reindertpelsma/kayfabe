#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""NVML sweep: every NVML field value id the driver knows, the common typed queries, `nvidia-smi -q -x` and every nvidia-smi query
property, recorded as rows (stdlib + ctypes; talks to libnvidia-ml.so.1 directly, no pynvml, no download).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md). Run natively on Vast boxes (traces/open_rows_ffdcgm_20261117/);
the ctypes layer is GPU-free tested against a fake libnvidia-ml built from source in the tests; not run in a kayfabe guest yet.

    nvml_sweep.py [--out FILE] [--lib PATH] [--field-max N] [--header nvml.h]

Field ids are not listed here: ids 0..N (default 1023) are all asked in one call each of 64, and the DRIVER's answer decides which exist
(per-field status INVALID_ARGUMENT = the library does not know that id). Labels come from nvml.h when one is found next to a CUDA toolkit
or in /usr/include (optional; an id without a label is still swept). The typed queries below use structs from the public nvml.h
(nvmlUtilization_t, nvmlMemory_t, nvmlBAR1Memory_t); everything else is plain unsigned values.
"""
import argparse
import ctypes
from ctypes import byref, c_char, c_int, c_longlong, c_uint, c_ulonglong, c_ubyte, c_void_p
import glob
from pathlib import Path
import re
import struct
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402
import nvml_lib as N  # noqa: E402

RC_NAMES = {0: "OK", 1: "UNINITIALIZED", 2: "INVALID_ARGUMENT", 3: "NOT_SUPPORTED", 4: "NO_PERMISSION", 5: "ALREADY_INITIALIZED", 6: "NOT_FOUND",
            7: "INSUFFICIENT_SIZE", 8: "INSUFFICIENT_POWER", 9: "DRIVER_NOT_LOADED", 10: "TIMEOUT", 15: "GPU_IS_LOST", 17: "OPERATING_SYSTEM",
            19: "IN_USE", 21: "NO_DATA", 24: "FREQ_NOT_SUPPORTED", 25: "ARGUMENT_VERSION_MISMATCH", 26: "DEPRECATED", 27: "NOT_READY",
            29: "INVALID_STATE", 999: "UNKNOWN"}
VALUE_FORMATS = {0: "d", 1: "I", 2: "Q", 3: "Q", 4: "q", 5: "i", 6: "H"}  # nvmlValueType_t: DOUBLE UINT ULONG ULONGLONG SLONGLONG INT USHORT


class FieldValue(ctypes.Structure):
    _fields_ = [("fieldId", c_uint), ("scopeId", c_uint), ("timestamp", c_longlong), ("latencyUsec", c_longlong), ("valueType", c_int),
                ("nvmlReturn", c_int), ("value", c_ubyte * 8)]


class Utilization(ctypes.Structure):
    _fields_ = [("gpu", c_uint), ("memory", c_uint)]


class Memory(ctypes.Structure):
    _fields_ = [("total", c_ulonglong), ("free", c_ulonglong), ("used", c_ulonglong)]


class Bar1(ctypes.Structure):
    _fields_ = [("total", c_ulonglong), ("free", c_ulonglong), ("used", c_ulonglong)]


U, Q, I = c_uint, c_ulonglong, c_int
# (key, symbols tried in order, extra unsigned arguments after the handle, output spec)
# output spec: list of ctypes types passed by reference | "str" (96-byte buffer) | "count" (a size-query: count only) | a Structure class
QUERIES = [
    ("utilization.rates", ["nvmlDeviceGetUtilizationRates"], [], Utilization),
    ("utilization.encoder", ["nvmlDeviceGetEncoderUtilization"], [], [U, U]),
    ("utilization.decoder", ["nvmlDeviceGetDecoderUtilization"], [], [U, U]),
    ("utilization.jpg", ["nvmlDeviceGetJpgUtilization"], [], [U, U]),
    ("utilization.ofa", ["nvmlDeviceGetOfaUtilization"], [], [U, U]),
    ("memory.info", ["nvmlDeviceGetMemoryInfo"], [], Memory),
    ("memory.bar1", ["nvmlDeviceGetBAR1MemoryInfo"], [], Bar1),
    ("memory.bus_width", ["nvmlDeviceGetMemoryBusWidth"], [], [U]),
    ("power.usage_mw", ["nvmlDeviceGetPowerUsage"], [], [U]),
    ("power.limit_mw", ["nvmlDeviceGetPowerManagementLimit"], [], [U]),
    ("power.limit_default_mw", ["nvmlDeviceGetPowerManagementDefaultLimit"], [], [U]),
    ("power.limit_enforced_mw", ["nvmlDeviceGetEnforcedPowerLimit"], [], [U]),
    ("power.limit_constraints_mw", ["nvmlDeviceGetPowerManagementLimitConstraints"], [], [U, U]),
    ("power.energy_mj", ["nvmlDeviceGetTotalEnergyConsumption"], [], [Q]),
    ("power.management_mode", ["nvmlDeviceGetPowerManagementMode"], [], [U]),
    ("perf.state", ["nvmlDeviceGetPerformanceState"], [], [U]),
    ("throttle.current", ["nvmlDeviceGetCurrentClocksEventReasons", "nvmlDeviceGetCurrentClocksThrottleReasons"], [], [Q]),
    ("throttle.supported", ["nvmlDeviceGetSupportedClocksEventReasons", "nvmlDeviceGetSupportedClocksThrottleReasons"], [], [Q]),
    ("temperature.gpu", ["nvmlDeviceGetTemperature"], [0], [U]),
    ("fan.count", ["nvmlDeviceGetNumFans"], [], [U]),
    ("fan.speed_pct", ["nvmlDeviceGetFanSpeed"], [], [U]),
    ("ecc.mode", ["nvmlDeviceGetEccMode"], [], [U, U]),
    ("pcie.link_gen_current", ["nvmlDeviceGetCurrPcieLinkGeneration"], [], [U]),
    ("pcie.link_width_current", ["nvmlDeviceGetCurrPcieLinkWidth"], [], [U]),
    ("pcie.link_gen_max", ["nvmlDeviceGetMaxPcieLinkGeneration"], [], [U]),
    ("pcie.link_width_max", ["nvmlDeviceGetMaxPcieLinkWidth"], [], [U]),
    ("pcie.link_gen_gpu_max", ["nvmlDeviceGetGpuMaxPcieLinkGeneration"], [], [U]),
    ("pcie.replay_counter", ["nvmlDeviceGetPcieReplayCounter"], [], [U]),
    ("encoder.stats", ["nvmlDeviceGetEncoderStats"], [], [U, U, U]),
    ("static.brand", ["nvmlDeviceGetBrand"], [], [U]),
    ("static.architecture", ["nvmlDeviceGetArchitecture"], [], [U]),
    ("static.cuda_cc", ["nvmlDeviceGetCudaComputeCapability"], [], [I, I]),
    ("static.num_cores", ["nvmlDeviceGetNumGpuCores"], [], [U]),
    ("static.compute_mode", ["nvmlDeviceGetComputeMode"], [], [U]),
    ("static.persistence_mode", ["nvmlDeviceGetPersistenceMode"], [], [U]),
    ("static.display_mode", ["nvmlDeviceGetDisplayMode"], [], [U]),
    ("static.display_active", ["nvmlDeviceGetDisplayActive"], [], [U]),
    ("static.mig_mode", ["nvmlDeviceGetMigMode"], [], [U, U]),
    ("static.virtualization_mode", ["nvmlDeviceGetVirtualizationMode"], [], [U]),
    ("static.accounting_mode", ["nvmlDeviceGetAccountingMode"], [], [U]),
    ("static.gsp_firmware_mode", ["nvmlDeviceGetGspFirmwareMode"], [], [U, U]),
    ("name", ["nvmlDeviceGetName"], [], "str"),
    ("vbios_version", ["nvmlDeviceGetVbiosVersion"], [], "str"),
    ("gsp_firmware_version", ["nvmlDeviceGetGspFirmwareVersion"], [], "str"),
    ("processes.compute", ["nvmlDeviceGetComputeRunningProcesses_v3", "nvmlDeviceGetComputeRunningProcesses_v2"], [], "count"),
    ("processes.graphics", ["nvmlDeviceGetGraphicsRunningProcesses_v3", "nvmlDeviceGetGraphicsRunningProcesses_v2"], [], "count"),
    ("processes.mps", ["nvmlDeviceGetMPSComputeRunningProcesses_v3", "nvmlDeviceGetMPSComputeRunningProcesses_v2"], [], "count"),
    ("encoder.sessions", ["nvmlDeviceGetEncoderSessions"], [], "count"),
]
for _t in range(4):
    for _key, _sym in (("clock.current", "nvmlDeviceGetClockInfo"), ("clock.max", "nvmlDeviceGetMaxClockInfo")):
        QUERIES.append(("%s.%s" % (_key, ("graphics", "sm", "mem", "video")[_t]), [_sym], [_t], [U]))
for _t in (0, 2):
    for _key, _sym in (("clock.app", "nvmlDeviceGetApplicationsClock"), ("clock.app_default", "nvmlDeviceGetDefaultApplicationsClock")):
        QUERIES.append(("%s.%s" % (_key, ("graphics", "sm", "mem")[_t]), [_sym], [_t], [U]))
for _t in range(4):
    QUERIES.append(("temperature.threshold.%d" % _t, ["nvmlDeviceGetTemperatureThreshold"], [_t], [U]))
for _d in (0, 1):
    QUERIES.append(("pcie.throughput_kbs.%s" % ("tx", "rx")[_d], ["nvmlDeviceGetPcieThroughput"], [_d], [U]))
for _e in (0, 1):
    for _c in (0, 1):
        QUERIES.append(("ecc.total_errors.%s_%s" % (("corrected", "uncorrected")[_e], ("volatile", "aggregate")[_c]),
                        ["nvmlDeviceGetTotalEccErrors"], [_e, _c], [Q]))
QUERIES.append(("nvlink.state.0", ["nvmlDeviceGetNvLinkState"], [0], [U]))
SYSTEM_QUERIES = [("system.driver_version", "nvmlSystemGetDriverVersion", "str"), ("system.nvml_version", "nvmlSystemGetNVMLVersion", "str"),
                  ("system.cuda_driver_version", "nvmlSystemGetCudaDriverVersion", [I])]


def rc_status(lib, rc):
    if rc in RC_NAMES:
        return RC_NAMES[rc]
    try:
        fn = lib.nvmlErrorString
        fn.restype = ctypes.c_char_p
        fn.argtypes = [c_int]
        return "ERR_%d_%s" % (rc, re.sub(r"\W+", "_", fn(rc).decode(errors="replace")).strip("_").upper())
    except (AttributeError, OSError):
        return "ERR_%d" % rc


def symbol(lib, names):
    for n in names:
        try:
            return getattr(lib, n), n
        except AttributeError:
            continue
    return None, names[0]


def fmt(v):
    return "%g" % v if isinstance(v, float) else str(v)


def run_query(lib, handle, key, syms, extras, out):
    fn, used = symbol(lib, syms)
    if fn is None:
        return N.row_line("query", key, syms[0], "NO_SYMBOL", "-")
    try:
        if out == "count":
            n = c_uint(0)
            fn.argtypes = [c_void_p, ctypes.POINTER(c_uint), c_void_p]
            rc = fn(handle, byref(n), None)
            return N.row_line("query", key, used, rc_status(lib, rc) if rc not in (0, 7) else "OK", "count=%d" % n.value)
        if out == "str":
            buf = ctypes.create_string_buffer(96)
            fn.argtypes = ([c_void_p] if handle is not None else []) + [ctypes.c_char_p, c_uint]
            rc = fn(*(([handle] if handle is not None else []) + [buf, 96]))
            val = census.scrub(buf.value.decode(errors="replace"), ips=False)
            return N.row_line("query", key, used, rc_status(lib, rc), val if rc == 0 else "-")
        outs = [out()] if isinstance(out, type) and issubclass(out, ctypes.Structure) else [t() for t in out]
        fn.argtypes = ([c_void_p] if handle is not None else []) + [c_uint] * len(extras) + [ctypes.POINTER(type(o)) for o in outs]
        rc = fn(*(([handle] if handle is not None else []) + list(extras) + [byref(o) for o in outs]))
        if rc != 0:
            return N.row_line("query", key, used, rc_status(lib, rc), "-")
        if isinstance(out, type) and issubclass(out, ctypes.Structure):
            val = ";".join("%s=%d" % (f, getattr(outs[0], f)) for f, _ in out._fields_)
        else:
            val = ";".join("v%d=%s" % (i, fmt(o.value)) for i, o in enumerate(outs))
            val = val.replace("v0=", "") if len(outs) == 1 else val
        return N.row_line("query", key, used, "OK", val)
    except (ctypes.ArgumentError, OSError) as e:
        return N.row_line("query", key, used, "ERR_CALL", str(e)[:80])


def decode_field(fv):
    fmt_c = VALUE_FORMATS.get(fv.valueType)
    if fmt_c is None:
        return None
    return struct.unpack("<" + fmt_c, bytes(fv.value)[:struct.calcsize("<" + fmt_c)])[0]


def sweep_fields(lib, handle, field_max, labels):
    rows, unknown = [], 0
    fn = lib.nvmlDeviceGetFieldValues
    fn.argtypes = [c_void_p, c_int, ctypes.POINTER(FieldValue)]
    for base in range(0, field_max + 1, 64):
        n = min(64, field_max + 1 - base)
        arr = (FieldValue * n)()
        for i in range(n):
            arr[i].fieldId = base + i
        rc = fn(handle, n, arr)
        if rc != 0:
            rows.append(N.row_line("field", "batch%d" % base, "-", rc_status(lib, rc), "-"))
            continue
        for fv in arr:
            if fv.nvmlReturn == 2:
                unknown += 1
                continue
            key = "fi%d" % fv.fieldId
            label = labels.get(fv.fieldId, "-")
            if fv.nvmlReturn != 0:
                rows.append(N.row_line("field", key, label, rc_status(lib, fv.nvmlReturn), "-"))
            else:
                v = decode_field(fv)
                rows.append(N.row_line("field", key, label, "OK" if v is not None else "ERR_VALUETYPE", "-" if v is None else fmt(v)))
    return rows, unknown


def find_header(explicit):
    cands = [explicit] if explicit else sorted(glob.glob("/usr/local/cuda*/targets/x86_64-linux/include/nvml.h") + glob.glob("/usr/local/cuda*/include/nvml.h")
                                              + ["/usr/include/nvml.h"], reverse=True)
    for c in cands:
        if c and Path(c).is_file():
            return c
    return None


def header_labels(path):
    labels = {}
    if not path:
        return labels
    for line in Path(path).read_text(errors="replace").splitlines():
        m = re.match(r"#define\s+(NVML_FI_[A-Z0-9_]+)\s+(\d+)\b", line)
        if m and m[1] != "NVML_FI_MAX":
            labels.setdefault(int(m[2]), m[1])
    return labels


def smi_text(args):
    try:
        p = subprocess.run(["nvidia-smi"] + args, capture_output=True, text=True, timeout=120, errors="replace")
        return p.returncode, p.stdout
    except (OSError, subprocess.TimeoutExpired) as e:
        return 127, str(e)


def sweep(lib_path, field_max, header):
    lib = ctypes.CDLL(lib_path)
    meta = []
    rows = []
    init = getattr(lib, "nvmlInit_v2", None) or lib.nvmlInit
    rc = init()
    if rc != 0:
        raise SystemExit("nvmlInit failed: %s" % rc_status(lib, rc))
    h = c_void_p()
    get = getattr(lib, "nvmlDeviceGetHandleByIndex_v2", None) or lib.nvmlDeviceGetHandleByIndex
    get.argtypes = [c_uint, ctypes.POINTER(c_void_p)]
    rc = get(0, byref(h))
    if rc != 0:
        raise SystemExit("no device 0: %s" % rc_status(lib, rc))
    labels = header_labels(find_header(header))
    frows, unknown = sweep_fields(lib, h, field_max, labels)
    rows += frows
    for key, syms, extras, out in QUERIES:
        rows.append(run_query(lib, h, key, syms, extras, out))
    for key, sym, out in SYSTEM_QUERIES:
        rows.append(run_query(lib, None, key, [sym], [], out))
    meta.append("field_scan=0..%d unknown_ids=%d known_ids=%d labels=%d" % (field_max, unknown, sum(1 for r in frows if r.startswith("field\tfi")), len(labels)))
    lib.nvmlShutdown()
    return rows, meta


def smi_sections():
    rows = []
    rc, xml = smi_text(["-q", "-x"])
    if rc == 0 and xml.strip().startswith("<"):
        try:
            rows += N.smi_rows(N.flatten_smi_xml(xml))
        except Exception as e:  # a parse problem is a row, not a crash
            rows.append(N.row_line("smi", "parse", "-", "ERR_PARSE", str(e)[:100]))
    else:
        rows.append(N.row_line("smi", "nvidia-smi -q -x", "-", "ERR_CALL", "rc=%d" % rc))
    rc, helptext = smi_text(["--help-query-gpu"])
    names = N.parse_help_query_gpu(helptext) if rc == 0 else []
    if names:
        rc, line = smi_text(["--query-gpu=" + ",".join(names), "--format=csv,noheader,nounits"])
        try:
            rows += N.smiq_rows(names, line.splitlines()[0]) if rc == 0 else []
        except (ValueError, IndexError) as e:
            rows.append(N.row_line("smiq", "all", "-", "ERR_PARSE", str(e)[:100]))
    return rows


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out")
    ap.add_argument("--lib", default="libnvidia-ml.so.1")
    ap.add_argument("--field-max", type=int, default=1023)
    ap.add_argument("--header")
    ap.add_argument("--no-smi", action="store_true")
    a = ap.parse_args(argv)
    rows, meta = sweep(a.lib, a.field_max, a.header)
    if not a.no_smi:
        rows += smi_sections()
    text = "".join("# %s\n" % m for m in meta) + "\n".join(rows) + "\n"
    if a.out:
        Path(a.out).write_text(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
