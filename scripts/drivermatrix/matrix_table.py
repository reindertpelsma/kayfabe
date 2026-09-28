#!/usr/bin/env python3
"""The host x guest matrix, per GPU architecture, DERIVED from the committed walk evidence (never typed).

Reads every queue log under traces/driver_matrix/walk/<box>/summary/ (q*.log, hostwalk*.log,
sweep*.log) and keeps, per (arch, host, guest), the LATEST revision's rows:
  MATRIX_ROW host=H guest=G rev=R FAST_SUITE_PASS=p ... ARMS=n   -> thin p/n
  LADDER host=H guest=G rev=R k/n                              -> ladder k/n
  GATES host=H V3_GATES_SUMMARY pass=p fail=f                    -> gates (per arch and host)
Queue logs that predate the host= field on LADDER lines (a guest walk on the bench host) are read
with the log's own `host=` from its *_START line.

★ [2026-09-28] THE THIRD AXIS — the GPU architecture (owner ruling C.5: every family Turing and
newer; `scripts/drivermatrix/sweep.sh` runs one box = one architecture). A line's arch is, in order:
  1. its own `arch=<die>` field (sweep.sh writes one on every result line),
  2. the `arch=` of the *_START line it follows (SWEEP_START / SWEEP_HOST_START),
  3. the box's ARCH file, <walk>/<box>/ARCH — the die recorded for a box whose logs predate the
     field (first non-comment line; the evidence it rests on is in the file's comments),
  4. "unknown".
One grid per arch; the per-arch grid is exactly the old grid when every row is one arch.

  usage: matrix_table.py [--arch DIE] [walk-dir]      (default: traces/driver_matrix/walk)
         matrix_table.py --selftest                   the committed logs must reproduce
                                                      V3_DRIVER_MATRIX.md §6.0's grid, and a sweep
                                                      row of another arch must land in its own grid
"""
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_WALK = os.path.join(HERE, "..", "..", "traces", "driver_matrix", "walk")
DOC = os.path.join(HERE, "..", "..", "docs", "design", "V3_DRIVER_MATRIX.md")
LOGS = re.compile(r"(q\d+\w*|hostwalk\d*|sweep\w*)\.log$")
ARCH_FIELD = re.compile(r"\sarch=(\S+)")


def vkey(v):
    return tuple(int(x) for x in re.findall(r"\d+", v))


def box_arch(walk, box):
    """The die in <walk>/<box>/ARCH (first non-comment, non-empty line), or None."""
    p = os.path.join(walk, box, "ARCH")
    if not os.path.isfile(p):
        return None
    for line in open(p, errors="replace"):
        line = line.split("#", 1)[0].strip()
        if line:
            return line.split()[0]
    return None


def parse(walk):
    """-> (cells, gates, boxes): cells[(arch, host, guest)][kind] = (order, rev, text),
    gates[(arch, host)] = (order, rev, text), boxes[arch] = {box, ...}."""
    cells, gates, boxes = {}, {}, {}

    def put(kind, arch, host, guest, rev, text, order):
        c = cells.setdefault((arch, host, guest), {})
        if kind not in c or c[kind][0] <= order:
            c[kind] = (order, rev, text)

    def put_gates(arch, host, text, order):
        if (arch, host) not in gates or gates[(arch, host)][0] <= order:
            gates[(arch, host)] = (order, "", text)

    for box in sorted(os.listdir(walk)):
        sdir = os.path.join(walk, box, "summary")
        if not os.path.isdir(sdir):
            continue
        default_arch = box_arch(walk, box) or "unknown"
        logs = sorted(f for f in os.listdir(sdir) if LOGS.match(f))
        for f in logs:
            start_host, start_ts, start_arch = None, "", None
            for n, line in enumerate(open(os.path.join(sdir, f), errors="replace")):
                m = re.match(r"\S+_START (\S+) rev=(\w+)(?: host=([\d.]+))?", line)
                if m:
                    start_ts, start_host = m.group(1), m.group(3)
                    a = ARCH_FIELD.search(line)
                    start_arch = a.group(1) if a else None
                order = (start_ts, n)
                a = ARCH_FIELD.search(line)
                arch = a.group(1) if a else (start_arch or default_arch)
                recorded = False
                m = re.match(r"V3_GATES_SUMMARY pass=(\d+) fail=(\d+)", line)
                if m and start_host:
                    put_gates(arch, start_host, f"{m.group(1)}/{int(m.group(1)) + int(m.group(2))}", order)
                    recorded = True
                m = None if recorded else re.match(
                    r"MATRIX_ROW host=([\d.]+) guest=([\d.]+) rev=(\w+) .*?(?:thin=(\d+/\d+)|FAST_SUITE_PASS=(\d+) .*ARMS=(\d+))", line)
                if m:
                    thin = m.group(4) or f"{m.group(5)}/{m.group(6)}"
                    put("thin", arch, m.group(1), m.group(2), m.group(3), thin, order)
                    recorded = True
                m = None if recorded else re.match(r"LADDER host=([\d.]+) guest=([\d.]+) rev=(\w+) (\d+/\d+)", line)
                if m:
                    put("ladder", arch, m.group(1), m.group(2), m.group(3), m.group(4), order)
                    recorded = True
                m = None if recorded else re.match(r"LADDER guest=([\d.]+) (UNSTAGED)", line)
                if m and start_host:
                    put("ladder", arch, start_host, m.group(1), "-", "unstaged", order)
                    recorded = True
                m = None if recorded else re.match(r"GATES host=([\d.]+) V3_GATES_SUMMARY pass=(\d+) fail=(\d+)", line)
                if m:
                    put_gates(arch, m.group(1), f"{m.group(2)}/{int(m.group(2)) + int(m.group(3))}", order)
                    recorded = True
                if recorded:
                    boxes.setdefault(arch, set()).add(box)
    return cells, gates, boxes


def render(cells, gates, arch):
    """The host x guest grid of one arch, as Markdown table lines (the pre-arch format, unchanged)."""
    hosts = sorted({h for a, h, _ in cells if a == arch} | {h for a, h in gates if a == arch}, key=vkey)
    guests = sorted({g for a, _, g in cells if a == arch}, key=vkey)
    out = ["| guest \\ host | " + " | ".join(hosts) + " |",
           "|---|" + "---|" * len(hosts),
           "| *gates* | " + " | ".join(gates.get((arch, h), ((), "", "—"))[2] for h in hosts) + " |"]
    for g in guests:
        row = []
        for h in hosts:
            c = cells.get((arch, h, g), {})
            parts = []
            if "thin" in c:
                parts.append(f"thin {c['thin'][2]}")
            if "ladder" in c:
                parts.append(f"ladder {c['ladder'][2]}")
            revs = sorted({v[1] for v in c.values() if v[1] and v[1] != "-"})
            row.append((", ".join(parts) + (f" `{'/'.join(revs)}`" if revs else "")) if parts else "")
        out.append(f"| {g} | " + " | ".join(row) + " |")
    return out


def archs_of(cells, gates):
    return sorted({a for a, _, _ in cells} | {a for a, _ in gates})


def tables(walk, only=None):
    cells, gates, boxes = parse(walk)
    out = []
    for arch in archs_of(cells, gates):
        if only and arch != only:
            continue
        if out:
            out.append("")
        out.append(f"#### arch `{arch}` — boxes: {', '.join(sorted(boxes.get(arch, ())))}")
        out.append("")
        out.extend(render(cells, gates, arch))
    return out


def doc_block(path):
    """V3_DRIVER_MATRIX.md §6.0's derived grids, as this script prints them: from the first
    `#### arch` heading through the last table line after it (headings, blank lines, table rows)."""
    lines = open(path, errors="replace").read().splitlines()
    for i, line in enumerate(lines):
        if line.startswith("#### arch `"):
            t = []
            for x in lines[i:]:
                if not (x.startswith("#### arch `") or x.startswith("|") or x == ""):
                    break
                t.append(x)
            while t and t[-1] == "":
                t.pop()
            return t
    return []


def selftest():
    fails = []

    def check(name, ok, detail=""):
        print(f"SELFTEST {name}: {'PASS' if ok else 'FAIL'}{(' — ' + detail) if detail else ''}")
        if not ok:
            fails.append(name)

    walk = DEFAULT_WALK
    cells, gates, boxes = parse(walk)
    archs = archs_of(cells, gates)
    check("no_row_without_an_arch", "unknown" not in archs, f"archs={archs}")
    # the two 2026-09-26 walk boxes are GA102 (their ARCH files: RTX 3090 0x2204, RTX 3080 Ti)
    check("the_walk_boxes_are_ga102", {"kfd", "kfh"} <= boxes.get("GA102", set()),
          f"GA102 boxes={sorted(boxes.get('GA102', ()))}")
    want = doc_block(DOC)
    got = tables(walk)
    check("doc_grid_found", len(want) > 3, f"{len(want)} lines in V3_DRIVER_MATRIX.md §6.0")
    check("committed_logs_reproduce_the_doc_grid", got == want,
          "" if got == want else "first difference: " + next(
              (f"doc {w!r} vs derived {g!r}" for w, g in zip(want, got) if w != g),
              f"{len(want)} vs {len(got)} lines"))
    ga102 = render(cells, gates, "GA102")

    # A sweep log of ANOTHER arch, in exactly the line formats sweep.sh emits, beside the committed
    # evidence: it must form its own grid and leave the GA102 grid untouched.
    with tempfile.TemporaryDirectory() as tmp:
        for box in os.listdir(walk):
            if os.path.isdir(os.path.join(walk, box)):
                os.symlink(os.path.join(os.path.abspath(walk), box), os.path.join(tmp, box))
        sdir = os.path.join(tmp, "_selftest_zz999", "summary")
        os.makedirs(sdir)
        with open(os.path.join(sdir, "sweep_zz999.log"), "w") as f:
            f.write("\n".join([
                "SWEEP_START 2026-09-29T01:00:00+00:00 rev=0123abcd tag=zz999 arch=ZZ999",
                'SWEEP_ARCH arch=ZZ999 src=pci.ids gpu="a synthetic GPU" pci=10de:ffff family="Synthetic" gpus=1 bdf=0000:01:00.0 lspci=""',
                "CLIENT_RC=0",
                "SWEEP_HOST_START 2026-09-29T01:10:00+00:00 rev=0123abcd host=580.159.04 arch=ZZ999",
                "SWAP host=580.159.04 already installed arch=ZZ999",
                "SWEEP_ROW_START 2026-09-29T01:10:01+00:00 row=gates:580.159.04",
                "GATES host=580.159.04 V3_GATES_SUMMARY pass=8 fail=1 failed: kf-gate9(rc=1,verdict=FAIL) arch=ZZ999",
                "SWEEP_ROW_EXIT 2026-09-29T01:20:00+00:00 row=gates:580.159.04 rc=1 secs=599",
                "MATRIX_ROW host=580.159.04 guest=580.159.04 rev=0123abcd thin=29/30 ladder=- arch=ZZ999",
                "LADDER host=580.159.04 guest=580.159.04 rev=0123abcd 4/4 arch=ZZ999",
                "LADDER guest=550.54.14 UNSTAGED arch=ZZ999",
                "SWEEP_HOST_START 2026-09-29T03:00:00+00:00 rev=0123abcd host=575.57.08 arch=ZZ999",
                "HOSTROW host=575.57.08 SWAP_FAILED arch=ZZ999",
                "SWEEP_EXIT 2026-09-29T05:00:00+00:00 rc=0 rows_run=5",
            ]) + "\n")
        c2, g2, b2 = parse(tmp)
        check("sweep_arch_is_its_own_grid", archs_of(c2, g2) == sorted(set(archs) | {"ZZ999"}),
              f"archs={archs_of(c2, g2)}")
        check("ga102_grid_untouched_by_another_arch", render(c2, g2, "GA102") == ga102)
        zz = render(c2, g2, "ZZ999")
        want_zz = [
            "| guest \\ host | 580.159.04 |",
            "|---|---|",
            "| *gates* | 8/9 |",
            "| 550.54.14 | ladder unstaged |",
            "| 580.159.04 | thin 29/30, ladder 4/4 `0123abcd` |",
        ]
        check("sweep_rows_parsed", zz == want_zz, "" if zz == want_zz else "\n" + "\n".join(zz))
        check("sweep_box_recorded", b2.get("ZZ999") == {"_selftest_zz999"}, f"{b2.get('ZZ999')}")
    print(f"SELFTEST {'PASS' if not fails else 'FAIL: ' + ', '.join(fails)}")
    return 0 if not fails else 1


def main(argv):
    if "--selftest" in argv:
        return selftest()
    only, walk, args = None, DEFAULT_WALK, list(argv)
    if "--arch" in args:
        i = args.index("--arch")
        only = args[i + 1]
        del args[i:i + 2]
    if args:
        walk = args[0]
    print("\n".join(tables(walk, only)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
