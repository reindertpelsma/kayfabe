#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""DCGM + NVML sweep row: run it on this machine, or compare a host run with a guest run (stdlib only).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md). Run natively on Vast boxes (traces/open_rows_ffdcgm_20261117/);
not run in a kayfabe guest yet: that is the next gate run.

    dcgm_nvml.py run --out DIR [--bundle DIR] [--diag-level 3] [--diag-timeout 1800] [--no-dcgm] [--emit]
    dcgm_nvml.py compare HOST GUEST          (each a run directory or a row log made with --emit)

`run` records, as rows (nvml_lib.py): every NVML field value id the driver knows, the typed NVML queries, `nvidia-smi -q -x` and every
nvidia-smi query property (nvml_sweep.py), then, when DCGM is installed, `dcgmi discovery`, every DCGM field of `dcgmi dmon -l`, and
`dcgmi diag -r 1` and `-r <level> -j`. DCGM is installed by provision.sh, never by this file; when `dcgmi` or `nv-hostengine` is missing the
DCGM part is NOT_RUN with the named reason in dcgm_status.txt (never skipped silently). Files: nvml_sweep.tsv, dcgm.tsv, dcgm_status.txt,
hostsmi.csv, summary.txt, raw/ (scrubbed command output). The last line is `NVML_DCGM_SUMMARY ...`; the exit status is 0 when the sweep ran.
`compare` applies the owner's rule: a number on the host must be a number in the guest, or the value is listed in
nvml_expected_not_supported.tsv with a reason; its exit status is 1 when there is a finding.
"""
import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402
import nvml_lib as N  # noqa: E402
import nvml_sweep as S  # noqa: E402
import rowio  # noqa: E402

CLASSES = HERE / "nvml_field_classes.tsv"
EXPECTED = HERE / "nvml_expected_not_supported.tsv"
DMON_BATCH = 16


def run(argv, timeout, env=None):
    t0 = time.time()
    try:
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", start_new_session=True,
                             stdin=subprocess.DEVNULL, env=env)
    except OSError as e:
        return 127, "cannot execute %s: %s" % (argv[0], e), 0.0
    try:
        out, _ = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return 124, (out or "") + "\ntimeout after %ds" % timeout, time.time() - t0
    return p.returncode, out or "", time.time() - t0


class Row:
    def __init__(self, a):
        self.a = a
        self.out = Path(a.out)
        self.raw = self.out / "raw"
        self.raw.mkdir(parents=True, exist_ok=True)
        self.bundle = Path(a.bundle) if a.bundle else None
        self.status = []
        self.dcgm_rows = []
        self.host_engine = None

    def save_raw(self, name, text):
        (self.raw / name).write_text(census.scrub(text)[-200000:])

    def locate(self, tool):
        found = shutil.which(tool)
        if found:
            return found
        if self.bundle:
            for sub in ("dcgm/root/usr/bin", "dcgm/usr/bin"):
                p = self.bundle / sub / tool
                if os.access(p, os.X_OK):
                    return str(p)
        return None

    def ensure_dcgm_files(self):
        """In a guest nothing is installed from the network: the bundle carries the files the box installed from NVIDIA's repository."""
        if shutil.which("dcgmi") or not self.bundle:
            return
        tarball = self.bundle / "dcgm_root.tar"
        if tarball.is_file() and os.geteuid() == 0:
            run(["tar", "-C", "/", "-xf", str(tarball)], 600)
            run(["ldconfig"], 60)

    def start_engine(self):
        he = self.locate("nv-hostengine")
        if not he:
            return "nv-hostengine not installed"
        log = str(self.raw / "nv-hostengine.log")
        self.host_engine = subprocess.Popen([he, "-n", "-f", log, "--log-level", "WARNING"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                            start_new_session=True)
        dcgmi = self.locate("dcgmi")
        for _ in range(30):
            if self.host_engine.poll() is not None:
                return "nv-hostengine exited rc=%s" % self.host_engine.returncode
            rc, text, _ = run([dcgmi, "discovery", "-l"], 20)
            self.save_raw("discovery.txt", text)
            if rc == 0 and N.parse_dcgm_discovery(text):
                return ""
            time.sleep(1)
        return "nv-hostengine did not answer discovery in 30 s"

    def stop_engine(self):
        if self.host_engine and self.host_engine.poll() is None:
            os.killpg(self.host_engine.pid, signal.SIGTERM)
            try:
                self.host_engine.wait(10)
            except subprocess.TimeoutExpired:
                os.killpg(self.host_engine.pid, signal.SIGKILL)

    def dmon_fields(self, dcgmi, fields, deadline):
        """Every field of `dcgmi dmon -l`, in groups. One field the GPU cannot watch makes the whole group fail ("Profiling is not supported
        for this group of GPUs", a native run), so a group with no table is split in halves until the refused field stands alone and gets its own
        NOT_SUPPORTED row."""
        if not fields:
            return []
        if time.time() > deadline:
            return [N.row_line("dcgm", n, "-", "NA", "not run: dmon time box") for _, n in fields]
        rc, text, _ = run([dcgmi, "dmon", "-e", ",".join(str(f) for f, _ in fields), "-c", "2", "-d", "300"], 60)
        series = N.parse_dcgm_dmon(text, [n for _, n in fields])
        if any(series.values()):
            return N.dmon_rows(series)
        if len(fields) == 1:
            return [N.dmon_error_row(fields[0][1], text)]
        mid = len(fields) // 2
        return self.dmon_fields(dcgmi, fields[:mid], deadline) + self.dmon_fields(dcgmi, fields[mid:], deadline)

    def dcgm_part(self):
        self.ensure_dcgm_files()
        dcgmi = self.locate("dcgmi")
        if not dcgmi:
            self.status.append("dcgm=NOT_RUN reason=dcgmi_not_installed")
            return
        err = self.start_engine()
        if err:
            self.status.append("dcgm=NOT_RUN reason=%s" % err.replace(" ", "_"))
            return
        rc, text, _ = run([dcgmi, "-v"], 20)
        self.save_raw("version.txt", text)
        ver = next((l.strip() for l in text.splitlines() if "version" in l.lower()), "-")
        n = N.parse_dcgm_discovery(open(self.raw / "discovery.txt").read())
        self.status.append("dcgm=RAN version=%s gpus=%s" % (ver.replace(" ", "_")[:60], n))
        self.dcgm_rows.append(N.row_line("dcgm", "discovery.gpus", "-", "OK" if n else "NA", str(n) if n else "-"))
        rc, text, _ = run([dcgmi, "dmon", "-l"], 60)
        self.save_raw("dmon_list.txt", text)
        fields = N.parse_dcgm_dmon_list(text)
        deadline = time.time() + 900
        for i in range(0, len(fields), DMON_BATCH):
            self.dcgm_rows += self.dmon_fields(dcgmi, fields[i:i + DMON_BATCH], deadline)
        level = self.a.diag_level
        for lvl in sorted({1, level}):
            rc, text, secs = run([dcgmi, "diag", "-r", str(lvl), "-j"], self.a.diag_timeout)
            self.save_raw("diag_r%d.json" % lvl, text)
            tests = N.parse_diag_json(text)
            self.status.append("diag_r%d rc=%d secs=%d tests=%d" % (lvl, rc, secs, len(tests)))
            if lvl == level:
                self.dcgm_rows += N.diag_rows(tests)
            else:
                self.dcgm_rows += [r.replace("diag\t", "diag\t[r%d] " % lvl, 1) for r in N.diag_rows(tests)]
        self.stop_engine()

    def run(self):
        stream = census.SmiStream().start()
        t0 = time.time()
        header = self.bundle / "cuda/include/nvml.h" if self.bundle else None   # labels only; the bundle carries the toolkit headers
        rows, meta = S.sweep(self.a.nvml_lib, 1023, str(header) if header and header.is_file() else None)
        rows += S.smi_sections()
        (self.out / "nvml_sweep.tsv").write_text("".join("# %s\n" % m for m in meta) + "\n".join(rows) + "\n")
        if not self.a.no_dcgm:
            self.dcgm_part()
        else:
            self.status.append("dcgm=NOT_RUN reason=disabled_by_flag")
        samples = stream.stop()
        t1 = time.time()
        (self.out / "hostsmi.csv").write_text(census.samples_to_csv(samples))
        (self.out / "dcgm.tsv").write_text("\n".join(self.dcgm_rows) + ("\n" if self.dcgm_rows else ""))
        (self.out / "dcgm_status.txt").write_text(census.scrub("\n".join(self.status) + "\n"))
        parsed = N.parse_rows((self.out / "nvml_sweep.tsv").read_text())
        count = lambda kind, st="OK": sum(1 for (k, _), r in parsed.items() if k == kind and r["status"] == st)
        total = lambda kind: sum(1 for (k, _) in parsed if k == kind)
        dparsed = N.parse_rows((self.out / "dcgm.tsv").read_text())
        dcount = lambda st: sum(1 for (k, _), r in dparsed.items() if k == "diag" and r["status"] == st)
        use, why = census.gpu_use_verdict(census.peaks(samples), "gpu")
        dstate = next((s.split()[0] for s in self.status if s.startswith("dcgm=")), "dcgm=NOT_RUN")
        line = ("NVML_DCGM_SUMMARY nvml_fields_ok=%d nvml_fields_other=%d queries_ok=%d/%d smi_ok=%d/%d smiq_na=%d %s dcgm_fields_ok=%d/%d "
                "diag_pass=%d diag_fail=%d diag_skip=%d gpu_use=%s secs=%d" % (
                    count("field"), total("field") - count("field"), count("query"), total("query"), count("smi"), total("smi"), count("smiq", "NA"),
                    dstate, sum(1 for (k, key), r in dparsed.items() if k == "dcgm" and not key.startswith("discovery.") and r["status"] == "OK"),
                    sum(1 for (k, key) in dparsed if k == "dcgm" and not key.startswith("discovery.")), dcount("PASS"), dcount("FAIL"), dcount("SKIP"), use, t1 - t0))
        (self.out / "summary.txt").write_text(census.scrub("# gpu_use_why=%s\n%s\n" % (why, "\n".join(self.status) + "\n" + line)))
        if self.a.emit:
            for name in ("nvml_sweep.tsv", "dcgm.tsv", "dcgm_status.txt", "summary.txt"):
                sys.stdout.write(rowio.emit(name, (self.out / name).read_text()))
        print(line)
        return 0 if count("field") + count("query") > 0 else 1


def cmd_compare(a):
    classes = N.load_classes(CLASSES.read_text())
    table = N.load_expected_ns(EXPECTED.read_text())
    hs, gs = rowio.load(a.host, "nvml_sweep.tsv"), rowio.load(a.guest, "nvml_sweep.tsv")
    if hs is None or gs is None:
        print("NVML_COMPARE error=missing_nvml_sweep_tsv host=%s guest=%s" % (hs is not None, gs is not None))
        return 2
    res, counts = N.compare_sweeps(N.parse_rows(hs), N.parse_rows(gs), classes, table)
    for r in res:
        if r["status"].startswith("FINDING") or r["status"] in ("EXPECTED_NS", "GUEST_ONLY_NUMBER", "OK_SERVED_THOUGH_LISTED", "TEXT_DIFF"):
            print("NVML_ROW %s %s %s %s" % (r["status"], r["kind"], r["key"], r["detail"]))
    n_find = N.findings(counts)
    hd, gd = rowio.load(a.host, "dcgm.tsv"), rowio.load(a.guest, "dcgm.tsv")
    dfind = 0
    if hd and gd:
        dres, dcounts = N.compare_sweeps({k: v for k, v in N.parse_rows(hd).items() if k[0] == "dcgm"},
                                         {k: v for k, v in N.parse_rows(gd).items() if k[0] == "dcgm"}, classes, table)
        for r in dres:
            if r["status"].startswith("FINDING") or r["status"] == "EXPECTED_NS":
                print("DCGM_ROW %s %s %s" % (r["status"], r["key"], r["detail"]))
        dfind += N.findings(dcounts)
        for key, h, g, v in N.compare_diag(N.parse_rows(hd), N.parse_rows(gd)):
            print("DIAG_ROW %s host=%s guest=%s %s" % (key, h, g, v))
            dfind += v.startswith("FINDING")
    print("NVML_COMPARE findings=%d dcgm_findings=%d %s" % (n_find, dfind, " ".join("%s=%d" % kv for kv in sorted(counts.items()))))
    return 1 if n_find + dfind else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--bundle")
    r.add_argument("--diag-level", type=int, default=3, choices=(1, 2, 3, 4))
    r.add_argument("--diag-timeout", type=int, default=1800)
    r.add_argument("--no-dcgm", action="store_true")
    r.add_argument("--emit", action="store_true")
    r.add_argument("--nvml-lib", default="libnvidia-ml.so.1", help="the NVML library to sweep (the tests point it at a fake)")
    c = sub.add_parser("compare")
    c.add_argument("host")
    c.add_argument("guest")
    a = ap.parse_args(argv)
    return Row(a).run() if a.cmd == "run" else cmd_compare(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
