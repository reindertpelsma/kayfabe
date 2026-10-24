#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""cursor_check.py - the grader of scripts/bench/display/cursor_lane.sh (owner ruling: the broker owns
all cursors, including the true XOR inversion; kayfabe composes none).

Inputs are images the lane captured: the HOST screencast with the pointer embedded (cursor_capture.py:
what the user sees), the GUEST frame (QEMU `screendump ... kf0`: what kayfabe produced), and the
guest's own cursor (xcursor.py image -> PAM with `# hot x y`, XFixes ARGB).

  hover --shot S.ppm --frame F.ppm --cursor C.pam --host X Y --guest-pointer GX GY [--out J]
        1. map:     the guest pattern's corner markers in S give origin and scale (host px per guest px);
        2. input:   the guest pointer equals the host pointer mapped into the guest (+-1.5 guest px):
                    host pointer -> broker -> kayfabe -> guest;
        3. kayfabe: the guest frame F has NO cursor in the cursor's box (it is the flat field there):
                    kayfabe composed nothing;
        4. broker:  S shows the guest's cursor image at the pointer: shape (IoU of the alpha mask with
                    the pixels that differ from the frame >= 0.9), position (best offset 0 +-1 host px),
                    colour (premultiplied-over composite within TOL at scale 1).
  grab  same inputs and checks as hover (the broker's overlay at the guest's own position).
  xor   --shot S.ppm --host X Y --frame-size W H --mask W H [--out J]
        the synthetic-VMM leg (nvkvm-broker-testclient --present WxH --shm --cursor-xor WxH): the
        testclient frame (green, red top-left quarter) located in S; every I-beam mask pixel at the
        pointer is the TRUE inversion of green (255,0,255), the pixels around it are green (no host
        pointer sprite, no black-and-white approximation).
  locate --shot S.ppm --frame-size W H     the guest pattern's mapping in S (JSON), exit 3 if absent
  locate-xor --shot S.ppm --frame-size W H --at U V   the testclient frame in S, and the host pixel of
                                           its pixel (U,V) (JSON), exit 3 if absent
  selftest
Exit: 0 PASS, 1 FAIL, 3 INCONCLUSIVE, 2 usage/input error.
"""
import json
import sys

FIELD = (100, 100, 100)
MARKERS = {"tl": (251, 3, 7), "tr": (3, 7, 251), "bl": (7, 251, 3), "br": (249, 249, 5)}
M = 24
TOL = 3
EXIT = {"PASS": 0, "FAIL": 1, "INCONCLUSIVE": 3}


# ---- images: (w, h, rows) with rows a list of lists of (r,g,b) or (r,g,b,a) ----

def read_pnm(path):
    with open(path, "rb") as f:
        data = f.read()
    if data.startswith(b"P6"):
        parts = data.split(maxsplit=4)
        w, h = int(parts[1]), int(parts[2])
        pix = parts[4][: w * h * 3]
        if len(pix) != w * h * 3:
            raise ValueError(f"{path}: truncated")
        return {"w": w, "h": h, "c": 3, "px": pix, "hot": None}
    if data.startswith(b"P7"):
        head, _, body = data.partition(b"ENDHDR\n")
        meta = {}
        hot = None
        for line in head.decode().splitlines()[1:]:
            if line.startswith("# hot"):
                hot = tuple(int(v) for v in line.split()[2:4])
            elif line.strip() and not line.startswith("#"):
                k, v = line.split(None, 1)
                meta[k] = v
        w, h, d = int(meta["WIDTH"]), int(meta["HEIGHT"]), int(meta["DEPTH"])
        # a cursor comes from the (untrusted) guest: bounded like the broker's own 256x256 plane
        if not (0 < w <= 512 and 0 < h <= 512) or hot is None or not (0 <= hot[0] < w and 0 <= hot[1] < h):
            raise ValueError(f"{path}: cursor {w}x{h} hot {hot} outside the bounds")
        if d != 4 or len(body) < w * h * 4:
            raise ValueError(f"{path}: not an RGB_ALPHA PAM")
        return {"w": w, "h": h, "c": 4, "px": body[: w * h * 4], "hot": hot}
    raise ValueError(f"{path}: not a P6 PPM or P7 PAM")


def at(img, x, y):
    c = img["c"]
    o = (y * img["w"] + x) * c
    return tuple(img["px"][o:o + c])


def near(a, b, tol):
    return all(abs(p - q) <= tol for p, q in zip(a, b))


def bbox(img, colour, tol=6):
    """bbox of the pixels of `colour`: exact first (a screencast of a lossless compositor is exact;
    bytes.find keeps a 4K frame fast), within `tol` per channel if no pixel is exact."""
    w = img["w"]
    key = bytes(colour)
    xs, ys = [], []
    for y in range(img["h"]):
        row = img["px"][y * w * 3:(y + 1) * w * 3]
        i = row.find(key)
        while i >= 0:
            if i % 3 == 0:
                xs.append(i // 3)
                ys.append(y)
            i = row.find(key, i + 1)
    if len(xs) < 16 and tol:
        for y in range(img["h"]):
            row = img["px"][y * w * 3:(y + 1) * w * 3]
            for x in range(w):
                o = x * 3
                if abs(row[o] - colour[0]) <= tol and abs(row[o + 1] - colour[1]) <= tol and abs(row[o + 2] - colour[2]) <= tol:
                    xs.append(x)
                    ys.append(y)
    if len(xs) < 16:
        return None
    return min(xs), min(ys), max(xs) + 1, max(ys) + 1


def find_marker(img, colour, region):
    """bbox of `colour` inside region (x0,y0,x1,y1) of img."""
    x0, y0, x1, y1 = region
    xs, ys = [], []
    for y in range(y0, y1):
        for x in range(x0, x1):
            if near(at(img, x, y), colour, 6):
                xs.append(x)
                ys.append(y)
    if len(xs) < 16:
        return None
    return min(xs), min(ys), max(xs) + 1, max(ys) + 1


def mapping(shot, gw, gh):
    """origin and scale of the guest frame inside the host shot, from the corner markers."""
    tl = bbox(shot, MARKERS["tl"])
    if tl is None:
        return None
    # search the other markers right of / below tl only
    tr = find_marker(shot, MARKERS["tr"], (tl[2], max(0, tl[1] - 4), shot["w"], min(shot["h"], tl[3] + 4)))
    bl = find_marker(shot, MARKERS["bl"], (max(0, tl[0] - 4), tl[3], min(shot["w"], tl[2] + 4), shot["h"]))
    if tr is None or bl is None:
        return None
    sx = (tr[0] - tl[0]) / (gw - M)
    sy = (bl[1] - tl[1]) / (gh - M)
    return {"ox": tl[0], "oy": tl[1], "sx": sx, "sy": sy}


def composite(base, cur, premultiplied=True, again=False):
    """cursor pixel (r,g,b,a) over base (r,g,b)."""
    r, g, b, a = cur
    if not premultiplied:
        r, g, b = (v * a // 255 for v in (r, g, b))
    if again:
        r, g, b = (v * a // 255 for v in (r, g, b))
    return tuple(min(255, c + base_c * (255 - a) // 255) for c, base_c in zip((r, g, b), base))


def grade_pointer(shot, frame, cursor, host, gptr, tol=TOL):
    res = {"tol": tol}
    mp = mapping(shot, frame["w"], frame["h"])
    res["mapping"] = mp
    if mp is None:
        res["verdict"], res["reason"] = "INCONCLUSIVE", "guest pattern markers not found in the host screencast"
        return res
    s = (mp["sx"] + mp["sy"]) / 2
    unit = abs(mp["sx"] - 1) < 0.01 and abs(mp["sy"] - 1) < 0.01
    res["scale"] = round(s, 4)
    # 2. input path
    want = ((host[0] - mp["ox"]) / mp["sx"], (host[1] - mp["oy"]) / mp["sy"])
    res["pointer_expected_guest"] = [round(v, 2) for v in want]
    res["pointer_guest"] = list(gptr)
    res["input_ok"] = abs(want[0] - gptr[0]) <= 1.5 and abs(want[1] - gptr[1]) <= 1.5
    # 3. kayfabe composed nothing: the frame is the flat field across the cursor's box
    hx, hy = cursor["hot"] or (0, 0)
    cx0, cy0 = gptr[0] - hx, gptr[1] - hy
    on = [(i, j) for j in range(cursor["h"]) for i in range(cursor["w"]) if at(cursor, i, j)[3] >= 128]
    if not on:
        res["verdict"], res["reason"] = "INCONCLUSIVE", "the guest cursor image is empty"
        return res
    inframe = [(cx0 + i, cy0 + j) for i, j in on if 0 <= cx0 + i < frame["w"] and 0 <= cy0 + j < frame["h"]]
    res["frame_cursor_pixels"] = sum(1 for (u, v) in inframe if not near(at(frame, u, v), FIELD, 2))
    res["kayfabe_composes_none"] = res["frame_cursor_pixels"] == 0
    # 4. the broker drew it: compare the shot around the pointer with the composite, best offset
    best = None
    for dy in range(-3, 4):
        for dx in range(-3, 4):
            maxd, sumd, n, hits, union = 0, 0, 0, 0, 0
            for j in range(-2, cursor["h"] + 2):
                for i in range(-2, cursor["w"] + 2):
                    u, v = cx0 + i, cy0 + j
                    if not (0 <= u < frame["w"] and 0 <= v < frame["h"]):
                        continue
                    X = round(mp["ox"] + (u + 0.5) * mp["sx"] - 0.5) + dx
                    Y = round(mp["oy"] + (v + 0.5) * mp["sy"] - 0.5) + dy
                    if not (0 <= X < shot["w"] and 0 <= Y < shot["h"]):
                        continue
                    base = at(frame, u, v)
                    inside = 0 <= i < cursor["w"] and 0 <= j < cursor["h"]
                    cpx = at(cursor, i, j) if inside else (0, 0, 0, 0)
                    exp = composite(base, cpx) if cpx[3] else base
                    got = at(shot, X, Y)
                    d = max(abs(p - q) for p, q in zip(got, exp))
                    maxd = max(maxd, d)
                    sumd += d
                    n += 1
                    differs = max(abs(p - q) for p, q in zip(got, base)) > 16
                    covered = cpx[3] >= 128
                    hits += differs and covered
                    union += differs or covered
            if n == 0:
                continue
            cand = {"dx": dx, "dy": dy, "max_diff": maxd, "mean_diff": round(sumd / n, 3), "iou": round(hits / union, 4) if union else 0.0}
            if best is None or (cand["mean_diff"], abs(dx) + abs(dy)) < (best["mean_diff"], abs(best["dx"]) + abs(best["dy"])):
                best = cand
    res["best"] = best
    if best is None:
        res["verdict"], res["reason"] = "INCONCLUSIVE", "the cursor box is outside the shot"
        return res
    # the 48/52 diagnostic of the 6d review: does the shot fit a SECOND premultiplication better?
    again = 0
    for i, j in on:
        u, v = cx0 + i, cy0 + j
        if 0 <= u < frame["w"] and 0 <= v < frame["h"]:
            X = round(mp["ox"] + (u + 0.5) * mp["sx"] - 0.5) + best["dx"]
            Y = round(mp["oy"] + (v + 0.5) * mp["sy"] - 0.5) + best["dy"]
            if 0 <= X < shot["w"] and 0 <= Y < shot["h"]:
                again = max(again, max(abs(p - q) for p, q in zip(at(shot, X, Y), composite(at(frame, u, v), at(cursor, i, j), again=True))))
    res["max_diff_if_premultiplied_twice"] = again
    res["position_ok"] = abs(best["dx"]) <= 1 and abs(best["dy"]) <= 1
    res["shape_ok"] = best["iou"] >= (0.9 if unit else 0.75)
    res["colour_ok"] = best["max_diff"] <= tol if unit else None
    ok = res["input_ok"] and res["kayfabe_composes_none"] and res["position_ok"] and res["shape_ok"] and res["colour_ok"] is not False
    res["verdict"] = "PASS" if ok else "FAIL"
    if not unit and ok:
        res["note"] = "scaled window: colour not graded (shape and position only)"
    return res


def ibeam(w, h):
    cx = w // 2
    return {(i, j) for j in range(h) for i in range(w) if i == cx or ((j == 0 or j == h - 1) and abs(i - cx) <= 2)}


def grade_xor(shot, host, frame_size, mask_size, tol=8):
    res = {"tol": tol}
    fw, fh = frame_size
    red = bbox(shot, (255, 0, 0))
    if red is None:
        res["verdict"], res["reason"] = "INCONCLUSIVE", "the testclient frame (red quarter) is not in the shot"
        return res
    s = (red[2] - red[0]) / (fw // 4)
    res["scale"] = round(s, 4)
    if s < 0.99:
        res["verdict"], res["reason"] = "INCONCLUSIVE", "the frame is shown below scale 1 (the patch is box-averaged)"
        return res
    ox, oy = red[0], red[1]
    gx, gy = (host[0] - ox) / s, (host[1] - oy) / s
    w, h = mask_size
    hx, hy = w // 2, h // 2
    mask = ibeam(w, h)
    bad_in, bad_out, n_in, n_out = 0, 0, 0, 0
    green, magenta = (0, 255, 0), (255, 0, 255)
    first = []
    for j in range(-3, h + 3):
        for i in range(-5, w + 5):
            u, v = int(gx) - hx + i, int(gy) - hy + j
            if not (fw // 4 <= u < fw and fh // 4 <= v < fh):  # stay on the green field
                continue
            X, Y = round(ox + (u + 0.5) * s - 0.5), round(oy + (v + 0.5) * s - 0.5)
            got = at(shot, X, Y)
            if (i, j) in mask:
                n_in += 1
                if not near(got, magenta, tol):
                    bad_in += 1
                    first.append(f"mask ({i},{j}) {got}")
            else:
                n_out += 1
                if not near(got, green, tol):
                    bad_out += 1
                    first.append(f"around ({i},{j}) {got}")
    res.update({"mask_px": n_in, "mask_bad": bad_in, "around_px": n_out, "around_bad": bad_out, "first": first[:5]})
    if n_in == 0:
        res["verdict"], res["reason"] = "INCONCLUSIVE", "the pointer is not over the green field"
        return res
    res["verdict"] = "PASS" if bad_in == 0 and bad_out == 0 else "FAIL"
    return res


# ---- selftest ----

def blank(w, h, col):
    return {"w": w, "h": h, "c": 3, "px": bytes(col) * (w * h), "hot": None}


def paint(img, x, y, col):
    c = img["c"]
    px = bytearray(img["px"])
    o = (y * img["w"] + x) * c
    px[o:o + c] = bytes(col)
    img["px"] = bytes(px)


def guest_frame(gw, gh):
    f = blank(gw, gh, FIELD)
    px = bytearray(f["px"])
    for key, (x0, y0) in {"tl": (0, 0), "tr": (gw - M, 0), "bl": (0, gh - M), "br": (gw - M, gh - M)}.items():
        for y in range(y0, y0 + M):
            for x in range(x0, x0 + M):
                o = (y * gw + x) * 3
                px[o:o + 3] = bytes(MARKERS[key])
    f["px"] = bytes(px)
    return f


def test_cursor():
    w, h = 12, 18
    px = bytearray(w * h * 4)
    for j in range(h):
        for i in range(w):
            if i <= j * w // h:
                a = 255 if i < j * w // h - 1 else 128
                v = 255 if i > 0 and j < h - 1 else 0
                pv = v * a // 255
                px[(j * w + i) * 4:(j * w + i) * 4 + 4] = bytes((pv, pv, pv, a))
    return {"w": w, "h": h, "c": 4, "px": bytes(px), "hot": (0, 0)}


def host_shot(frame, cursor, gptr, origin, sw, sh, offset=(0, 0), again=False, show=True):
    shot = blank(sw, sh, (30, 30, 30))
    px = bytearray(shot["px"])
    ox, oy = origin
    for v in range(frame["h"]):
        row = frame["px"][v * frame["w"] * 3:(v + 1) * frame["w"] * 3]
        o = ((oy + v) * sw + ox) * 3
        px[o:o + frame["w"] * 3] = row
    shot["px"] = bytes(px)
    if show:
        hx, hy = cursor["hot"]
        for j in range(cursor["h"]):
            for i in range(cursor["w"]):
                c = at(cursor, i, j)
                if c[3]:
                    u, v = gptr[0] - hx + i, gptr[1] - hy + j
                    X, Y = ox + u + offset[0], oy + v + offset[1]
                    paint(shot, X, Y, composite(at(frame, u, v), c, again=again))
    return shot


def selftest():
    fails = 0

    def check(name, cond):
        nonlocal fails
        print(f"CURSOR_SELFTEST {name} {'PASS' if cond else 'FAIL'}")
        fails += 0 if cond else 1

    gw, gh = 320, 200
    frame = guest_frame(gw, gh)
    cur = test_cursor()
    gptr = (150, 90)
    org = (37, 21)
    host = (org[0] + gptr[0], org[1] + gptr[1])
    good = grade_pointer(host_shot(frame, cur, gptr, org, 400, 260), frame, cur, host, gptr)
    check("hover_good_passes", good["verdict"] == "PASS" and good["kayfabe_composes_none"])
    off = grade_pointer(host_shot(frame, cur, gptr, org, 400, 260, offset=(3, 0)), frame, cur, host, gptr)
    check("cursor_3px_off_fails_position", off["verdict"] == "FAIL" and not off["position_ok"])
    twice = grade_pointer(host_shot(frame, cur, gptr, org, 400, 260, again=True), frame, cur, host, gptr)
    check("double_premultiplication_fails_colour", twice["verdict"] == "FAIL" and twice["colour_ok"] is False
          and twice["max_diff_if_premultiplied_twice"] <= TOL)
    gone = grade_pointer(host_shot(frame, cur, gptr, org, 400, 260, show=False), frame, cur, host, gptr)
    check("no_host_cursor_fails_shape", gone["verdict"] == "FAIL" and not gone["shape_ok"])
    composed = guest_frame(gw, gh)
    for j in range(cur["h"]):
        for i in range(cur["w"]):
            if at(cur, i, j)[3] >= 128:
                paint(composed, gptr[0] + i, gptr[1] + j, (255, 255, 255))
    kf = grade_pointer(host_shot(frame, cur, gptr, org, 400, 260), composed, cur, host, gptr)
    check("cursor_in_kayfabe_frame_fails", kf["verdict"] == "FAIL" and not kf["kayfabe_composes_none"])
    wrong_input = grade_pointer(host_shot(frame, cur, gptr, org, 400, 260), frame, cur, (host[0] + 5, host[1]), gptr)
    check("guest_pointer_elsewhere_fails_input", wrong_input["verdict"] == "FAIL" and not wrong_input["input_ok"])
    # XOR leg
    fw, fh = 320, 240
    shot = blank(500, 400, (30, 30, 30))
    o = (60, 50)
    for v in range(fh):
        for u in range(fw):
            paint(shot, o[0] + u, o[1] + v, (255, 0, 0) if (u < fw // 4 and v < fh // 4) else (0, 255, 0))
    hp = (o[0] + 200, o[1] + 150)
    xor_shot = {**shot}
    for (i, j) in ibeam(9, 16):
        paint(xor_shot, hp[0] - 4 + i, hp[1] - 8 + j, (255, 0, 255))
    check("xor_true_inversion_passes", grade_xor(xor_shot, hp, (fw, fh), (9, 16))["verdict"] == "PASS")
    approx = {**shot}
    for (i, j) in ibeam(9, 16):
        paint(approx, hp[0] - 4 + i, hp[1] - 8 + j, (0, 0, 0))
    check("xor_black_approximation_fails", grade_xor(approx, hp, (fw, fh), (9, 16))["verdict"] == "FAIL")
    check("xor_missing_fails", grade_xor(shot, hp, (fw, fh), (9, 16))["verdict"] == "FAIL")
    print(f"CURSOR_SELFTEST={'PASS' if fails == 0 else 'FAIL'} ({fails})")
    return 0 if fails == 0 else 1


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd, args = argv[1], argv[2:]
    if cmd == "selftest":
        return selftest()

    def opt(name, n=1):
        i = args.index("--" + name)
        vals = args[i + 1:i + 1 + n]
        return vals[0] if n == 1 else vals

    try:
        if cmd in ("hover", "grab"):
            res = grade_pointer(read_pnm(opt("shot")), read_pnm(opt("frame")), read_pnm(opt("cursor")),
                                tuple(float(v) for v in opt("host", 2)), tuple(int(float(v)) for v in opt("guest-pointer", 2)))
        elif cmd == "locate":
            fw, fh = (int(v) for v in opt("frame-size", 2))
            mp = mapping(read_pnm(opt("shot")), fw, fh)
            print(json.dumps(mp))
            return 0 if mp else 3
        elif cmd == "locate-xor":
            fw, fh = (int(v) for v in opt("frame-size", 2))
            u, v = (int(x) for x in opt("at", 2))
            img = read_pnm(opt("shot"))
            red = bbox(img, (255, 0, 0))
            if red is None:
                print(json.dumps(None))
                return 3
            s = (red[2] - red[0]) / (fw // 4)
            print(json.dumps({"ox": red[0], "oy": red[1], "scale": round(s, 4),
                              "host": [round(red[0] + (u + 0.5) * s - 0.5), round(red[1] + (v + 0.5) * s - 0.5)]}))
            return 0
        elif cmd == "xor":
            res = grade_xor(read_pnm(opt("shot")), tuple(float(v) for v in opt("host", 2)),
                            tuple(int(v) for v in opt("frame-size", 2)), tuple(int(v) for v in opt("mask", 2)))
        else:
            print(f"unknown subcommand {cmd}", file=sys.stderr)
            return 2
    except (OSError, ValueError, KeyError, IndexError) as e:
        print(f"cursor_check: {e}", file=sys.stderr)
        return 2
    res["mode"] = cmd
    res["classification"] = "MEASURED"
    if "--out" in args:
        with open(opt("out"), "w") as f:
            f.write(json.dumps(res) + "\n")
    print(f"CURSOR_GRADE mode={cmd} verdict={res['verdict']} " + " ".join(
        f"{k}={res[k]}" for k in ("scale", "input_ok", "kayfabe_composes_none", "position_ok", "shape_ok", "colour_ok",
                                  "max_diff_if_premultiplied_twice", "mask_bad", "around_bad", "reason") if k in res))
    return EXIT[res["verdict"]]


if __name__ == "__main__":
    sys.exit(main(sys.argv))
