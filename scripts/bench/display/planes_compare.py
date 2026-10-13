#!/usr/bin/env python3
"""planes_compare.py <shot.ppm> <ref.ppm> <what> — compare the kf3 console's screendump of
`kfplanes show <what>` with the reference frame `kfplanes ppm` computes (BT.709 limited range for
the YUV bars). Prints one CMP line: the whole-frame and overlay-rectangle mismatch counts at a
tolerance, the largest channel error, and, for the YUV formats, which conversion the console matches
best (BT.709 / BT.601, limited / full, U/V order) — the console-composition (L01) evidence.
Pure python, no dependencies; untrusted input is only the guest-made image, read with bounds."""
import sys


def read_ppm(path):
    with open(path, "rb") as f:
        data = f.read()
    parts, i = [], 0
    while len(parts) < 4:
        while data[i:i + 1].isspace():
            i += 1
        if data[i:i + 1] == b"#":
            while data[i:i + 1] not in (b"\n", b""):
                i += 1
            continue
        j = i
        while not data[j:j + 1].isspace():
            j += 1
        parts.append(data[i:j])
        i = j
    i += 1
    w, h, mx = int(parts[1]), int(parts[2]), int(parts[3])
    if parts[0] != b"P6" or mx != 255 or w <= 0 or h <= 0 or w * h > 8192 * 8192:
        raise ValueError("unsupported PPM")
    px = data[i:i + 3 * w * h]
    if len(px) != 3 * w * h:
        raise ValueError("short PPM")
    return w, h, px


BAR_Y = [235, 219, 188, 173, 78, 63, 32, 16]
BAR_U = [128, 16, 154, 42, 214, 102, 240, 128]
BAR_V = [128, 138, 16, 26, 230, 240, 118, 128]


def conv(Y, U, V, kr, kb, limited):
    if limited:
        y, u, v = (Y - 16) * 255 / 219, (U - 128) * 255 / 224, (V - 128) * 255 / 224
    else:
        y, u, v = Y, U - 128, V - 128
    kg = 1 - kr - kb
    r = y + 2 * (1 - kr) * v
    b = y + 2 * (1 - kb) * u
    g = (y - kr * r - kb * b) / kg
    c = lambda t: max(0, min(255, round(t)))
    return c(r), c(g), c(b)


def main():
    shot, ref, what = sys.argv[1], sys.argv[2], sys.argv[3]
    sw, sh, sp = read_ppm(shot)
    rw, rh, rp = read_ppm(ref)
    if (sw, sh) != (rw, rh):
        print(f"CMP what={what} verdict=FAIL why=size shot={sw}x{sh} ref={rw}x{rh}")
        return
    W, H = sw, sh
    ox, oy = W // 8, H // 8
    ow, oh = (960, 540) if what == "nv12-scaled" else (640, 360)
    ow, oh = min(ow, W - ox), min(oh, H - oy)
    tol = 6
    bad = bad_in = 0
    maxerr = 0
    for y in range(H):
        row = 3 * W * y
        for x in range(W):
            k = row + 3 * x
            e = max(abs(sp[k] - rp[k]), abs(sp[k + 1] - rp[k + 1]), abs(sp[k + 2] - rp[k + 2]))
            if e > tol:
                bad += 1
                if ox <= x < ox + ow and oy <= y < oy + oh:
                    bad_in += 1
            maxerr = max(maxerr, e)
    line = f"CMP what={what} size={W}x{H} tol={tol} mismatched={bad} in_overlay={bad_in} of {ow*oh} max_err={maxerr}"
    if what != "rgb":
        # sample the centre of each bar (one row, middle of the overlay) and score candidate conversions
        cands = {}
        for name, kr, kb in (("709", 0.2126, 0.0722), ("601", 0.299, 0.114), ("2020", 0.2627, 0.0593)):
            for lim in (True, False):
                for swap in (False, True):
                    err = 0
                    for b in range(8):
                        cx = ox + int((b + 0.5) * ow / 8)
                        cy = oy + oh // 2
                        k = 3 * (W * cy + cx)
                        U, V = (BAR_V[b], BAR_U[b]) if swap else (BAR_U[b], BAR_V[b])
                        r, g, bb = conv(BAR_Y[b], U, V, kr, kb, lim)
                        err = max(err, abs(sp[k] - r), abs(sp[k + 1] - g), abs(sp[k + 2] - bb))
                    cands[f"{name}{'L' if lim else 'F'}{'-swapUV' if swap else ''}"] = err
        best = min(cands, key=cands.get)
        samples = []
        for b in range(8):
            cx = ox + int((b + 0.5) * ow / 8)
            k = 3 * (W * (oy + oh // 2) + cx)
            samples.append(f"{sp[k]:02x}{sp[k+1]:02x}{sp[k+2]:02x}")
        line += f" best_match={best}(err {cands[best]}) bt709L_err={cands['709L']} bars={','.join(samples)}"
    verdict = "PASS" if bad_in == 0 and bad == 0 else ("OVERLAY_DIFF" if bad_in else "OUTSIDE_DIFF")
    print(line + f" verdict={verdict}")


if __name__ == "__main__":
    main()
