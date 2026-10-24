#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""hue_vibrance_check.py - the grader of scripts/bench/windows/hue_vibrance_lane.sh (PR #92, automated).

Reads QEMU screendumps (binary P6 PPM of the kf0 console = what kayfabe's colour kernel produced)
and the run's qemu.log, and decides, for one Hue / Digital Vibrance setting:

  pixels     every test-pattern patch equals the prediction from the head colour program the guest
             driver really programmed (the last `kf3: display: colour program #N` line, PR #92
             automated checks) through an independent reference of the output stage, within TOL;
  live       the pattern's frame counter differs between two screendumps taken >= 1 s apart (the
             console is still being updated: run 4500 froze at 0 fps);
  no_fault   no new `scanout REFUSED SDR colour program` line (the Fault::Console of run 4500) and no
             new `display: STALL` / GPU fault line since the setting was applied.

The prediction assumes the guest's ILUT and OLUT are identity (byte k -> k/256 -> k). The BASELINE
screendump (neutral settings, identity OCSC0) checks that assumption first; if the baseline does not
reproduce the pattern within TOL the pixel verdict is INCONCLUSIVE (tables not identity), never PASS.

Subcommands:
  grade  --baseline B.ppm --shot S1.ppm --shot2 S2.ppm --qemu-log LOG --since BYTES [--out J]
  health --qemu-log LOG --since BYTES                      (liveness/refusal counters only)
  predict --line 'kf3: display: colour program #..'        (print the predicted patches)
  selftest                                                 (synthetic cases; no VM)
Exit: 0 PASS, 1 FAIL, 3 INCONCLUSIVE, 2 usage/input error.
"""
import json
import math
import re
import sys

# The pattern (hue_vibrance_guest.ps1 $Patches; change both together).
PATCHES = [
    (255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0), (0, 255, 255), (255, 0, 255),
    (192, 64, 64), (64, 192, 64), (64, 64, 192), (224, 160, 128), (96, 160, 224), (160, 96, 32),
    (0, 0, 0), (64, 64, 64), (128, 128, 128), (192, 192, 192), (255, 255, 255), (32, 200, 96),
]
BACKGROUND = (128, 128, 128)
SIZE = (1920, 1080)
COUNTER = (660, 940, 600, 120)  # x, y, w, h
TOL = 2

# The kf-qemu line (crates/kf-qemu/src/display.rs `program_line`; change both together).
PROGRAM_RE = re.compile(
    r"kf3: display: colour program #(?P<n>\d+) head=(?P<head>\d+) ocsc0=\[(?P<m>-?\d+(?:,-?\d+){11})\] "
    r"olut=(?P<olut>\d+|bypass) interp=(?P<interp>[01]) mirror=(?P<mirror>[01]) "
    r"signed_behind_mirror=(?P<signed>[01])"
)
REFUSED_RE = re.compile(r"scanout REFUSED SDR colour program")
FAULT_RE = re.compile(r"display: STALL|scanout halted|Fault::Gpu|display worker (?:died|exited)")


def patch_origin(k):
    return 160 + 280 * (k % 6), 120 + 280 * (k // 6)


# ---- the output stage, independently (V3_SDR_COLOR.md reading; odd extension [inferred]) ----

def table_value(table, x, c, interpolate):
    """table: list of (r,g,b) UNORM16 samples, or None = identity; x clamped to [0, 1]."""
    if table is None:
        return min(max(x, 0.0), 1.0)
    n = len(table)
    x = 0.0 if math.isnan(x) else min(max(x, 0.0), 1.0)
    pos = x * (n - 1)
    i = min(int(math.floor(pos)), n - 2)
    a, b = table[i][c] / 65536.0, table[i + 1][c] / 65536.0
    return a + (b - a) * (pos - i) if interpolate else a


def quantise(y):
    y = 0.0 if math.isnan(y) else min(max(y, 0.0), 1.0)
    return min(int(math.floor(y * 256.0)), 255)


def output(src, m, table=None, interpolate=False, mirror=False, extension="odd"):
    """One pixel: src in [0,1]^3, m the encoded 3x4 (65536 = 1.0)."""
    out = []
    for c in range(3):
        v = sum(src[i] * m[c * 4 + i] / 65536.0 for i in range(3)) + m[c * 4 + 3] / 65536.0
        if mirror and v < 0:
            y = -table_value(table, -v, c, interpolate) if extension == "odd" else table_value(table, -v, c, interpolate)
        else:
            y = table_value(table, v, c, interpolate)
        out.append(quantise(y))
    return tuple(out)


def predict(m, mirror):
    """Expected console bytes of every patch under identity ILUT/OLUT (byte k -> k/256)."""
    return [output(tuple(v / 256.0 for v in p), m, None, False, mirror) for p in PATCHES]


# ---- inputs ----

def read_ppm(path):
    with open(path, "rb") as f:
        data = f.read()
    # P6 <w> <h> <max> <single whitespace> <bytes>; no comments in QEMU's
    parts = data.split(maxsplit=4)
    if len(parts) < 5 or parts[0] != b"P6" or int(parts[3]) != 255:
        raise ValueError(f"{path}: not a binary 8-bit PPM")
    w, h = int(parts[1]), int(parts[2])
    pix = parts[4][: w * h * 3]
    if len(pix) != w * h * 3:
        raise ValueError(f"{path}: truncated ({len(pix)} of {w * h * 3} bytes)")
    return w, h, pix


def px(img, x, y):
    w, _, pix = img
    o = (y * w + x) * 3
    return pix[o], pix[o + 1], pix[o + 2]


def patch_mean(img, k, inset=40):
    x0, y0 = patch_origin(k)
    acc = [0, 0, 0]
    n = 0
    for y in range(y0 + inset, y0 + 200 - inset, 4):
        for x in range(x0 + inset, x0 + 200 - inset, 4):
            p = px(img, x, y)
            acc = [a + b for a, b in zip(acc, p)]
            n += 1
    return tuple(round(a / n) for a in acc)


def region_bytes(img, rect):
    x0, y0, w, h = rect
    return bytes(b for y in range(y0, y0 + h) for x in range(x0, x0 + w) for b in px(img, x, y))


def last_program(log_text, head=0):
    last = None
    for mt in PROGRAM_RE.finditer(log_text):
        if int(mt.group("head")) == head:
            last = mt
    if not last:
        return None
    return {
        "n": int(last.group("n")),
        "matrix": [int(v) for v in last.group("m").split(",")],
        "olut": last.group("olut"),
        "interp": last.group("interp") == "1",
        "mirror": last.group("mirror") == "1",
        "signed_behind_mirror": last.group("signed") == "1",
    }


def read_log(path, since):
    with open(path, "rb") as f:
        f.seek(since)
        tail = f.read().decode("utf-8", "replace")
    with open(path, "rb") as f:
        whole = f.read().decode("utf-8", "replace")
    return whole, tail


def health(tail):
    return {
        "refused_colour_lines": len(REFUSED_RE.findall(tail)),
        "fault_lines": len(FAULT_RE.findall(tail)),
    }


# ---- grading ----

def grade(baseline, shot, shot2, log_whole, log_tail, tol=TOL):
    res = {"tol": tol}
    for name, img in (("baseline", baseline), ("shot", shot), ("shot2", shot2)):
        if (img[0], img[1]) != SIZE:
            res["verdict"] = "INCONCLUSIVE"
            res["reason"] = f"{name} is {img[0]}x{img[1]}, the pattern is {SIZE[0]}x{SIZE[1]}"
            return res
    # the identity-table assumption, on the baseline
    base_err = max(max(abs(a - b) for a, b in zip(patch_mean(baseline, k), p)) for k, p in enumerate(PATCHES))
    bg_err = max(abs(a - b) for a, b in zip(px(baseline, 40, 40), BACKGROUND))
    res["baseline_max_err"] = max(base_err, bg_err)
    prog = last_program(log_whole)
    res["program"] = prog
    h = health(log_tail)
    res.update(h)
    res["live"] = region_bytes(shot, COUNTER) != region_bytes(shot2, COUNTER)
    pixel = "INCONCLUSIVE"
    worst = None
    if prog is None:
        res["pixel_reason"] = "no colour program line in qemu.log (kf3 older than the PR #92 checks?)"
    elif res["baseline_max_err"] > tol:
        res["pixel_reason"] = f"baseline differs from the pattern by {res['baseline_max_err']} (> {tol}): ILUT/OLUT not identity"
    else:
        want = predict(prog["matrix"], prog["mirror"])
        errs = []
        for k, w in enumerate(want):
            got = patch_mean(shot, k)
            e = max(abs(a - b) for a, b in zip(got, w))
            errs.append({"patch": k, "pattern": PATCHES[k], "want": w, "got": got, "err": e})
        worst = max(errs, key=lambda e: e["err"])
        res["patches"] = errs
        pixel = "PASS" if worst["err"] <= tol else "FAIL"
        res["pixel_reason"] = f"worst patch {worst['patch']} err {worst['err']}"
    res["pixel"] = pixel
    if not res["live"] or h["refused_colour_lines"] or h["fault_lines"] or pixel == "FAIL":
        res["verdict"] = "FAIL"
    elif pixel == "INCONCLUSIVE":
        res["verdict"] = "INCONCLUSIVE"
    else:
        res["verdict"] = "PASS"
    return res


EXIT = {"PASS": 0, "FAIL": 1, "INCONCLUSIVE": 3}


# ---- selftest: synthetic screendumps and logs, no VM ----

def synth(m, mirror, counter, tables_identity=True):
    w, h = SIZE
    buf = bytearray(bytes(BACKGROUND) * (w * h))
    want = predict(m, mirror) if tables_identity else [tuple(min(255, v + 9) for v in p) for p in PATCHES]
    for k, col in enumerate(want):
        x0, y0 = patch_origin(k)
        row = bytes(col) * 200
        for y in range(y0, y0 + 200):
            o = (y * w + x0) * 3
            buf[o:o + 600] = row
    x0, y0, cw, ch = COUNTER
    for y in range(y0, y0 + ch):
        o = (y * w + x0) * 3
        buf[o:o + cw * 3] = bytes([255 if (x + counter) % 7 else 0 for x in range(cw) for _ in range(3)])
    return (w, h, bytes(buf))


def selftest():
    fails = 0

    def check(name, cond):
        nonlocal fails
        print(f"HV_SELFTEST {name} {'PASS' if cond else 'FAIL'}")
        fails += 0 if cond else 1

    # the reference: the four hand-computed cases of kf-cuda/examples/color_mirror.rs
    lifted = [(16384 + i * 32,) * 3 for i in range(1025)]
    neg = [65536, -32768, 0, 0, 0, 65536, 0, 0, 0, 0, 65536, 0]
    lum = (0.2126, 0.7152, 0.0722)
    vib = [0] * 12
    for r in range(3):
        for c in range(3):
            vib[r * 4 + c] = round(((1 - 1.5) * lum[c] + (1.5 if r == c else 0)) * 65536)
    check("hand_mirror_negative", output((0.25, 0.75, 0), neg, lifted, False, True) == (0, 160, 64))
    check("hand_unmirrored_negative", output((0.25, 0.75, 0), neg, lifted, False, False) == (64, 160, 64))
    check("hand_mirror_vibrance", output((1, 0, 0), vib, lifted, True, True) == (192, 0, 0))
    check("hand_unmirrored_vibrance", output((1, 0, 0), vib, lifted, True, False) == (192, 64, 64))
    check("even_extension_differs", output((1, 0, 0), vib, lifted, True, True, "even") != (192, 0, 0))
    ident = [65536, 0, 0, 32, 0, 65536, 0, 32, 0, 0, 65536, 32]
    check("identity_reproduces_pattern", predict(ident, True) == PATCHES)
    line = "kf3: display: colour program #{} head=0 ocsc0=[{}] olut=1025 interp=0 mirror=1 signed_behind_mirror={} (x)"
    hue = [round(v * 65536) for v in (0.7, 0.5, -0.2, 0, -0.1, 0.9, 0.2, 0, 0.3, -0.4, 1.1, 0)]
    hue[3] = hue[7] = hue[11] = 32
    log = line.format(1, ",".join(map(str, ident)), 0) + "\n" + line.format(2, ",".join(map(str, hue)), 1) + "\n"
    check("program_line_parses", last_program(log)["matrix"] == hue and last_program(log)["n"] == 2)
    base = synth(ident, True, 0)
    good = grade(base, synth(hue, True, 1), synth(hue, True, 2), log, "")
    check("good_setting_passes", good["verdict"] == "PASS")
    frozen = grade(base, synth(hue, True, 1), synth(hue, True, 1), log, "")
    check("frozen_console_fails", frozen["verdict"] == "FAIL" and not frozen["live"])
    wrong = grade(base, synth(ident, True, 1), synth(ident, True, 2), log, "")
    check("unapplied_matrix_fails", wrong["verdict"] == "FAIL" and wrong["pixel"] == "FAIL")
    refused = grade(base, synth(hue, True, 1), synth(hue, True, 2), log,
                    "kf3: display: scanout REFUSED SDR colour program: mirrored OLUT behind a signed matrix\n")
    check("refusal_line_fails", refused["verdict"] == "FAIL")
    lifted_base = synth(ident, True, 0, tables_identity=False)
    inc = grade(lifted_base, synth(hue, True, 1), synth(hue, True, 2), log, "")
    check("non_identity_tables_inconclusive", inc["verdict"] == "INCONCLUSIVE")
    noline = grade(base, synth(hue, True, 1), synth(hue, True, 2), "", "")
    check("no_program_line_inconclusive", noline["verdict"] == "INCONCLUSIVE")
    print(f"HV_SELFTEST={'PASS' if fails == 0 else 'FAIL'} ({fails})")
    return 0 if fails == 0 else 1


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd, args = argv[1], argv[2:]
    opt = {}
    i = 0
    while i < len(args):
        if args[i].startswith("--") and i + 1 < len(args):
            opt[args[i][2:]] = args[i + 1]
            i += 2
        else:
            print(f"unexpected argument {args[i]}", file=sys.stderr)
            return 2
    try:
        if cmd == "selftest":
            return selftest()
        if cmd == "predict":
            m = PROGRAM_RE.search(opt["line"])
            prog = last_program(opt["line"])
            if not m or not prog:
                print("no colour program line", file=sys.stderr)
                return 2
            for p, w in zip(PATCHES, predict(prog["matrix"], prog["mirror"])):
                print(f"HV_PREDICT pattern={p} console={w}")
            return 0
        if cmd == "health":
            _, tail = read_log(opt["qemu-log"], int(opt.get("since", "0")))
            print(json.dumps(health(tail)))
            return 0
        if cmd == "grade":
            whole, tail = read_log(opt["qemu-log"], int(opt.get("since", "0")))
            res = grade(read_ppm(opt["baseline"]), read_ppm(opt["shot"]), read_ppm(opt["shot2"]), whole, tail,
                        int(opt.get("tol", TOL)))
            res["classification"] = "MEASURED"
            text = json.dumps(res)
            if "out" in opt:
                with open(opt["out"], "w") as f:
                    f.write(text + "\n")
            print(f"HV_GRADE verdict={res['verdict']} pixel={res.get('pixel')} live={res.get('live')} "
                  f"refused={res.get('refused_colour_lines')} faults={res.get('fault_lines')} "
                  f"baseline_err={res.get('baseline_max_err')} {res.get('pixel_reason', res.get('reason', ''))}")
            return EXIT[res["verdict"]]
    except (OSError, ValueError, KeyError) as e:
        print(f"hue_vibrance_check: {e}", file=sys.stderr)
        return 2
    print(f"unknown subcommand {cmd}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
