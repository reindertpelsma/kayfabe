#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Derive the utilisation-reporting layouts from NVIDIA's open headers. STATUS: LIVE, 2026-11-17.

`docs/design/V3_UTILIZATION_FLAG.md`. The compiler is the parser (owner rule 2026-09-21, the same rule
`tools/derive_hwref.sh` follows): `probe.c` is compiled against one ogkm tree's public SDK headers and prints
`sizeof` / `offsetof` / macro values. Nothing is typed by hand, and a tag with no receipt is UNMEASURED: the
Rust side refuses it by name (exact membership, never a nearest neighbour).

  generate.py all   --work DIR     fetch every tag (sparse, shallow), compile, write the receipts and the Rust
  generate.py emit                 receipts -> crates/kf-abi/src/utilization_generated.rs
  generate.py check --ogkm-580 P --ogkm-595 Q
                                   recompile the two PINNED oracle trees (scripts/ci/fetch_oracles.sh) and fail
                                   unless they equal their receipts, and the committed Rust equals `emit`
                                   (what crates/kf-abi/tests/utilization_layout_is_current.rs runs)

Receipts: traces/util_layout_20261117/<tag>.tsv + sources.json (commit, probe hash, output hash).
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROBE = ROOT / "tools/util-probe/probe.c"
RECEIPTS = ROOT / "traces/util_layout_20261117"
OUT = ROOT / "crates/kf-abi/src/utilization_generated.rs"
URL = "https://github.com/NVIDIA/open-gpu-kernel-modules"
# The tags worth a row: every measured driver-matrix tag from the first one that has the GPUMON v2 geometry
# (570.86.15) up. RUSD's own layout moves between tags (580.159.04 -> 595.84 moves perfDevUtil), so each tag is
# probed on its own and a tag nobody probed is refused.
TAGS = """570.86.15 570.124.06 570.148.08 575.51.02 575.51.03 575.57.08 575.64.05 580.65.06 580.82.07 580.94.02
580.95.05 580.105.08 580.126.09 580.159.04 580.173.02 590.48.01 595.84 595.91.07 610.43.02 610.57.04 615.71.09""".split()
INC = ("src/common/sdk/nvidia/inc",)

# TSV key -> Rust field of `Layout`. Every field is a number the compiler printed.
FIELDS = [
    ("value", "NV2080_CTRL_CMD_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2", "gpumon_cmd"),
    ("value", "NV2080_CTRL_GPUMON_SAMPLE_TYPE_PERFMON_UTIL", "gpumon_type_util"),
    ("value", "NV2080_GPUMON_PID_INVALID", "pid_invalid"),
    ("value", "NV2080_CTRL_PERF_GPUMON_SAMPLE_COUNT_PERFMON_UTIL", "sample_count"),
    ("sizeof", "NV2080_CTRL_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2_PARAMS", "params_size"),
    ("field", "NV2080_CTRL_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2_PARAMS.type", "off_type"),
    ("field", "NV2080_CTRL_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2_PARAMS.bufSize", "off_buf_size"),
    ("field", "NV2080_CTRL_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2_PARAMS.count", "off_count"),
    ("field", "NV2080_CTRL_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2_PARAMS.tracker", "off_tracker"),
    ("field", "NV2080_CTRL_PERF_GET_GPUMON_PERFMON_UTIL_SAMPLES_V2_PARAMS.samples", "off_samples"),
    ("sizeof", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE", "sample_size"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.base.timeStamp", "off_timestamp"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.fb", "off_fb"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.gr", "off_gr"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.nvenc", "off_nvenc"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.nvdec", "off_nvdec"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.nvjpg", "off_nvjpg"),
    ("field", "NV2080_CTRL_PERF_GPUMON_PERFMON_UTIL_SAMPLE.nvofa", "off_nvofa"),
    ("sizeof", "NV2080_CTRL_PERF_GPUMON_ENGINE_UTIL_SAMPLE", "engine_size"),
    ("field", "NV2080_CTRL_PERF_GPUMON_ENGINE_UTIL_SAMPLE.util", "eng_off_util"),
    ("field", "NV2080_CTRL_PERF_GPUMON_ENGINE_UTIL_SAMPLE.vgpuScale", "eng_off_vgpu_scale"),
    ("field", "NV2080_CTRL_PERF_GPUMON_ENGINE_UTIL_SAMPLE.procId", "eng_off_proc_id"),
    ("field", "NV2080_CTRL_PERF_GPUMON_ENGINE_UTIL_SAMPLE.subProcessID", "eng_off_sub_proc_id"),
    ("value", "NV2080_CTRL_CMD_INTERNAL_INIT_USER_SHARED_DATA", "rusd_init_cmd"),
    ("value", "NV2080_CTRL_CMD_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL", "rusd_poll_cmd"),
    ("value", "RM_USER_SHARED_DATA", "rusd_class"),
    ("sizeof", "NV2080_CTRL_INTERNAL_INIT_USER_SHARED_DATA_PARAMS", "init_size"),
    ("field", "NV2080_CTRL_INTERNAL_INIT_USER_SHARED_DATA_PARAMS.bInit", "init_off_binit"),
    ("field", "NV2080_CTRL_INTERNAL_INIT_USER_SHARED_DATA_PARAMS.physAddr", "init_off_phys"),
    ("sizeof", "NV2080_CTRL_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL_PARAMS", "poll_size"),
    ("field", "NV2080_CTRL_INTERNAL_USER_SHARED_DATA_SET_DATA_POLL_PARAMS.polledDataMask", "poll_off_mask"),
    ("value", "NV00DE_RUSD_POLL_PERF", "poll_perf"),
    ("value", "RUSD_TIMESTAMP_INVALID", "ts_invalid"),
    ("value", "RUSD_TIMESTAMP_WRITE_IN_PROGRESS", "ts_in_progress"),
    ("value", "RUSD_ENG_UTILIZATION_VID_ENG_NVENC", "eng_nvenc"),
    ("value", "RUSD_ENG_UTILIZATION_VID_ENG_NVDEC", "eng_nvdec"),
    ("value", "RUSD_ENG_UTILIZATION_VID_ENG_NVJPG", "eng_nvjpg"),
    ("value", "RUSD_ENG_UTILIZATION_VID_ENG_NVOFA", "eng_nvofa"),
    ("value", "RUSD_ENG_UTILIZATION_COUNT", "eng_count"),
    ("sizeof", "NV00DE_SHARED_DATA", "rusd_size"),
    ("field", "NV00DE_SHARED_DATA.perfDevUtil", "rusd_off_util"),
    ("sizeof", "RUSD_PERF_DEVICE_UTILIZATION", "util_size"),
    ("field", "RUSD_PERF_DEVICE_UTILIZATION.lastModifiedTimestamp", "util_off_ts"),
    ("field", "RUSD_PERF_DEVICE_UTILIZATION.info", "util_off_info"),
    ("field", "RUSD_PERF_DEVICE_UTILIZATION_INFO.gpuPercentBusy", "info_off_gpu"),
    ("field", "RUSD_PERF_DEVICE_UTILIZATION_INFO.memoryPercentBusy", "info_off_mem"),
    ("field", "RUSD_PERF_DEVICE_UTILIZATION_INFO.engUtil", "info_off_eng"),
    ("sizeof", "RUSD_ENG_UTILIZATION", "eng_util_size"),
    ("field", "RUSD_ENG_UTILIZATION.clkPercentBusy", "eng_util_off_busy"),
    ("field", "RUSD_ENG_UTILIZATION.samplingPeriodUs", "eng_util_off_period"),
]


def sha(b):
    return hashlib.sha256(b).hexdigest()


def probe(tree):
    """Compile probe.c against `tree` and return its stdout. gcc is the parser."""
    inc = [f"-I{tree}/{i}" for i in INC] + [f"-I{tree}/{INC[0]}/class"]
    with tempfile.TemporaryDirectory() as d:
        exe = os.path.join(d, "probe")
        r = subprocess.run(["gcc", "-Wall", "-Werror", *inc, str(PROBE), "-o", exe], capture_output=True, text=True)
        if r.returncode != 0:
            return None, [l for l in r.stderr.splitlines() if "error:" in l][:1]
        return subprocess.run([exe], check=True, capture_output=True).stdout, None


def parse(raw):
    vals = {}
    for line in raw.decode().splitlines():
        kind, name, value = line.split("\t")
        vals[(kind, name)] = int(value)
    return vals


def fetch(tag, dest):
    dest.mkdir(parents=True, exist_ok=True)
    run = lambda *a: subprocess.run(["git", "-C", str(dest), *a], check=True, capture_output=True, text=True)
    if not (dest / ".git").exists():
        run("init", "-q")
        run("remote", "add", "o", URL)
    run("fetch", "-q", "--depth", "1", "--no-tags", "o", f"refs/tags/{tag}:refs/tags/{tag}")
    run("sparse-checkout", "init", "--cone")
    run("sparse-checkout", "set", *INC)
    run("checkout", "-q", "-f", tag)
    return run("rev-parse", "HEAD").stdout.strip()


def cmd_all(work):
    RECEIPTS.mkdir(parents=True, exist_ok=True)
    sources = {}
    for tag in TAGS:
        tree = Path(work) / tag
        commit = fetch(tag, tree)
        raw, why = probe(tree)
        if raw is None:
            # A tag whose headers do not have the shapes this feature reads (an older RUSD init struct, say) gets
            # no row, and the Rust side refuses it by name. The reason is kept, not hidden.
            sources[tag] = {"commit": commit, "unprobeable": why[0].split("error:")[1].strip()}
            print("UNPROBEABLE", tag, why)
            continue
        (RECEIPTS / f"{tag}.tsv").write_bytes(raw)
        sources[tag] = {"commit": commit, "probe_sha256": sha(PROBE.read_bytes()), "output_sha256": sha(raw)}
        print("probed", tag, commit[:12])
    (RECEIPTS / "sources.json").write_text(json.dumps(sources, indent=1, sort_keys=True) + "\n")
    cmd_emit()


def load_receipts():
    sources = json.loads((RECEIPTS / "sources.json").read_text())
    rows = []
    for tag in sorted(sources, key=lambda t: tuple(int(x) for x in t.split("."))):
        if "unprobeable" in sources[tag]:
            continue
        raw = (RECEIPTS / f"{tag}.tsv").read_bytes()
        assert sha(raw) == sources[tag]["output_sha256"], f"receipt {tag} does not match sources.json"
        assert sources[tag]["probe_sha256"] == sha(PROBE.read_bytes()), f"receipt {tag} was probed by another probe.c"
        v = parse(raw)
        rows.append((tag, tuple(v[(k, n)] for k, n, _ in FIELDS)))
    return rows


def emit_text():
    rows = load_receipts()
    runs = []  # consecutive-or-not identical value vectors share a row
    for tag, vec in rows:
        for r in runs:
            if r[1] == vec:
                r[0].append(tag)
                break
        else:
            runs.append(([tag], vec))
    names = [n for _, _, n in FIELDS]
    out = [
        "// @generated by tools/util-probe/generate.py from public ogkm headers (receipts: traces/util_layout_20261117).",
        "// Do not edit. Exact membership: a driver version with no row here has no layout and is refused by name.",
        "#![allow(missing_docs)]",
        "use crate::DriverVersion;",
        "",
        "/// What the compiler printed for one ogkm tag (or several tags that printed the same thing).",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub struct Layout {",
    ]
    out += [f"    pub {n}: u64," for n in names]
    out += ["}", "", "/// `(versions that printed this layout, the layout)`.", "pub const ROWS: &[(&[DriverVersion], Layout)] = &["]
    for tags, vec in runs:
        vs = ", ".join(
            "DriverVersion { major: %d, minor: %d, patch: %d }" % (tuple(int(x) for x in (t.split(".") + ["0"])[:3]))
            for t in tags
        )
        out.append("    (&[%s], Layout {" % vs)
        out += [f"        {n}: {v}," for n, v in zip(names, vec)]
        out.append("    }),")
    out.append("];")
    return "\n".join(out) + "\n", [t for t, _ in rows]


def rustfmt(path):
    subprocess.run(["rustfmt", "--edition", "2024", str(path)], check=True)


def cmd_emit():
    text, tags = emit_text()
    OUT.write_text(text)
    rustfmt(OUT)
    print("emitted", OUT.relative_to(ROOT), len(tags), "tags")


def cmd_check(o580, o595):
    sources = json.loads((RECEIPTS / "sources.json").read_text())
    for tag, tree in (("580.159.04", o580), ("595.84", o595)):
        raw, _ = probe(tree)
        want = (RECEIPTS / f"{tag}.tsv").read_bytes()
        if raw != want:
            raise SystemExit(f"{tag}: a fresh probe of {tree} differs from its receipt: regenerate (generate.py all)")
        assert sha(raw) == sources[tag]["output_sha256"]
    text, _ = emit_text()
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "u.rs"
        p.write_text(text)
        rustfmt(p)
        if p.read_text() != OUT.read_text():
            raise SystemExit("utilization_generated.rs is not what the receipts emit: run generate.py emit")
    print("UTIL_LAYOUT_CURRENT")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("all")
    a.add_argument("--work", required=True)
    sub.add_parser("emit")
    c = sub.add_parser("check")
    c.add_argument("--ogkm-580", required=True)
    c.add_argument("--ogkm-595", required=True)
    args = ap.parse_args()
    if args.cmd == "all":
        cmd_all(args.work)
    elif args.cmd == "emit":
        cmd_emit()
    else:
        cmd_check(args.ogkm_580, args.ogkm_595)


if __name__ == "__main__":
    sys.exit(main())
