#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""Mutation check for the XOR cursor: break the sources on purpose, one defect
at a time, and demand that test-xor notices -- at least TWO distinct failing
checks per mutant (one failing check is a coincidence; two is a net).

Each mutant is an exact textual replacement in a COPY of nb_xor.c, nb_frame.c
or nb_cursor.c (the real files are never touched).  A mutant whose text is not
found is itself an error, so the table cannot rot silently when the code moves.

    python3 test/mutate_xor.py            # all mutants
    python3 test/mutate_xor.py -v         # also print the failing checks
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)

# (name, file, old text, new text)
MUTANTS = [
    # --- the state machine ---------------------------------------------------
    ("hover XOR composed leaves the host pointer visible", "nb_xor.c",
     "            o.ptr = NB_XP_HIDDEN;\n            o.ov = NB_XO_XOR;",
     "            o.ptr = NB_XP_GUEST_IMAGE;\n            o.ov = NB_XO_XOR;"),
    ("hover XOR readable draws no overlay", "nb_xor.c",
     "            o.ptr = NB_XP_HIDDEN;\n            o.ov = NB_XO_XOR;",
     "            o.ptr = NB_XP_HIDDEN;\n            o.ov = NB_XO_OFF;"),
    ("the enter serial is ignored", "nb_xor.c",
     "bool pa = in->ptr_inside && in->serial_ok;",
     "bool pa = in->ptr_inside;"),
    ("the pointer being outside is ignored", "nb_xor.c",
     "bool pa = in->ptr_inside && in->serial_ok;",
     "bool pa = in->serial_ok;"),
    ("an unreadable frame hides the host pointer (no cursor at all)", "nb_xor.c",
     "            o.ptr = NB_XP_GUEST_IMAGE;     /* the approximation, as before */",
     "            o.ptr = NB_XP_HIDDEN;"),
    ("grab with a readable frame still draws the approximation", "nb_xor.c",
     "o.ov = in->frame_readable ? NB_XO_XOR : NB_XO_IMAGE;",
     "o.ov = NB_XO_IMAGE;"),
    ("grab draws before the first MOVE", "nb_xor.c",
     "if (in->kind == NB_XK_NONE || !in->placed) {",
     "if (in->kind == NB_XK_NONE) {"),
    ("an arrow in hover is no longer the host pointer", "nb_xor.c",
     "    case NB_XK_PLAIN:\n        o.ptr = NB_XP_GUEST_IMAGE;",
     "    case NB_XK_PLAIN:\n        o.ptr = NB_XP_HIDDEN;"),
    ("no cursor leaves the host pointer alone", "nb_xor.c",
     "    case NB_XK_NONE:\n        o.ptr = NB_XP_HIDDEN;",
     "    case NB_XK_NONE:\n        o.ptr = NB_XP_UNTOUCHED;"),
    ("the choice is composed even when the frame is unreadable", "nb_xor.c",
     "             : in->frame_readable      ? NB_XC_COMPOSED : NB_XC_APPROX;",
     "             : NB_XC_COMPOSED;"),
    ("the choice announced on every evaluation (a line per frame)", "nb_xor.c",
     "    if (!ctx->choice_valid || ctx->last_choice != ch ||",
     "    if (1 || !ctx->choice_valid || ctx->last_choice != ch ||"),
    # --- placement / pointer map ----------------------------------------------
    ("placement truncates instead of rounding", "nb_xor.c",
     "    return floor_div(2 * a * (int64_t)n + (int64_t)d, 2 * (int64_t)d);",
     "    return floor_div(a * (int64_t)n, (int64_t)d);"),
    ("the right edge is not rounded on its own (size from the left edge)", "nb_xor.c",
     "    x1 = round_ratio((int64_t)tlx + pw, nx, dx);",
     "    x1 = x0 + round_ratio((int64_t)pw, nx, dx);"),
    ("the pointer map does not clamp", "nb_xor.c",
     "    if (q > (int64_t)frame_dim - 1) { q = (int64_t)frame_dim - 1; }",
     ""),
    # --- the frame reader -------------------------------------------------------
    ("a memfd without F_SEAL_SHRINK is read anyway", "nb_frame.c",
     "            !(seals & F_SEAL_SHRINK)) {",
     "            0) {"),
    ("the row end is not checked against the mapping", "nb_frame.c",
     "        if (end > (uint64_t)f->len) {",
     "        if (0) {"),
    ("the rectangle may run past the frame", "nb_frame.c",
     "        (uint64_t)rx + rw > f->w || (uint64_t)ry + rh > f->h) {",
     "        0) {"),
    ("the per-frame description check is skipped", "nb_frame.c",
     "    if (f->w != g->w || f->h != g->h || f->stride != g->stride ||\n"
     "        f->offset != g->offset || f->fourcc != g->fourcc) {",
     "    if (0) {"),
    ("any exporter is taken for system memory", "nb_frame.c",
     "    return exp_name &&\n"
     "           (strcmp(exp_name, \"udmabuf\") == 0",
     "    return exp_name && (1 ||\n"
     "           strcmp(exp_name, \"udmabuf\") == 0"),
    ("a tiled modifier is accepted", "nb_frame.c",
     "    return modifier == NB_FRAME_MOD_LINEAR || modifier == NB_FRAME_MOD_INVALID;",
     "    return 1;"),
    ("the description may exceed the buffer", "nb_frame.c",
     "    if (need > size || need > (uint64_t)INT32_MAX) {",
     "    if (0) {"),
    ("a frame is mapped read-write", "nb_frame.c",
     "    m = mmap(NULL, (size_t)need, PROT_READ, MAP_SHARED, is_shm ? fd : sfd, 0);",
     "    m = mmap(NULL, (size_t)need, PROT_READ | PROT_WRITE, MAP_SHARED, is_shm ? fd : sfd, 0);"),
    ("the shrinking scaler samples one pixel (a thin serif vanishes)", "nb_cursor.c",
     "        uint32_t sy1 = dh >= sh ? sy0 + 1u",
     "        uint32_t sy1 = 1 ? sy0 + 1u"),
    ("the shrinking scaler does not average columns", "nb_cursor.c",
     "            uint32_t sx1 = dw >= sw ? sx0 + 1u",
     "            uint32_t sx1 = 1 ? sx0 + 1u"),
    # --- the compose --------------------------------------------------------------
    ("the patch ORs instead of XORs", "nb_cursor.c",
     "((frame[(size_t)fy * stride_px + (size_t)fx] ^ m) & 0x00ffffffu);\n"
     "            n++;\n"
     "        }\n"
     "    }\n"
     "    return n;\n"
     "}\n"
     "\n"
     "size_t nb_cursor_xor_patch",
     "((frame[(size_t)fy * stride_px + (size_t)fx] | m) & 0x00ffffffu);\n"
     "            n++;\n"
     "        }\n"
     "    }\n"
     "    return n;\n"
     "}\n"
     "\n"
     "size_t nb_cursor_xor_patch"),
    ("the pointer is not offset by the hot spot", "nb_xor.c",
     "            tlx = (int64_t)v->ptr_x - (int64_t)c->hot_x + c->bx;",
     "            tlx = (int64_t)v->ptr_x + c->bx;"),
    ("a failed read does not fall back", "nb_xor.c",
     "                r->why = w;\n                in.frame_readable = false;\n                r->act = nb_xor_decide(&in);",
     "                r->why = w;"),
    ("the rectangle origin is not clipped to the frame", "nb_xor.c",
     "        x0 = tlx < 0 ? 0 : tlx;",
     "        x0 = tlx;"),
    ("new pixels under the cursor are not noticed", "nb_xor.c",
     "memcmp(ctx->att, out, n * sizeof out[0]) != 0;",
     "0;"),
]


def build_and_run(workdir):
    cc = os.environ.get("CC", "cc")
    exe = os.path.join(workdir, "t")
    cmd = [cc, "-O1", "-g", "-std=gnu11", "-w", "-D_GNU_SOURCE", "-I", workdir,
           "-I", os.path.join(workdir, "common"), "-o", exe,
           os.path.join(workdir, "test_xor.c"), os.path.join(workdir, "nb_xor.c"),
           os.path.join(workdir, "nb_frame.c"), os.path.join(workdir, "nb_cursor.c")]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr
    try:
        r = subprocess.run([exe], capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return ["TIMEOUT"], ""
    fails = [l for l in r.stdout.splitlines() if l.startswith("FAIL ")]
    if r.returncode != 0 and not fails:
        fails = ["CRASH rc=%d" % r.returncode]
    return fails, ""


def main():
    verbose = "-v" in sys.argv
    bad = 0
    print("mutation check: %d mutants" % len(MUTANTS))
    for i, (name, fname, old, new) in enumerate(MUTANTS, 1):
        with tempfile.TemporaryDirectory(prefix="mutxor") as tmp:
            for f in ("nvkvm_broker.h", "nb_xor.c", "nb_frame.c", "nb_cursor.c"):
                shutil.copy(os.path.join(SRC, f), tmp)
            shutil.copytree(os.path.join(SRC, "common"), os.path.join(tmp, "common"))
            shutil.copy(os.path.join(HERE, "test_xor.c"), tmp)
            path = os.path.join(tmp, fname)
            text = open(path).read()
            if text.count(old) != 1:
                print("  ERROR  #%d %s: mutation text found %d times in %s"
                      % (i, name, text.count(old), fname))
                bad += 1
                continue
            open(path, "w").write(text.replace(old, new))
            fails, err = build_and_run(tmp)
            if fails is None:
                print("  ERROR  #%d %s: does not compile\n%s" % (i, name, err[:400]))
                bad += 1
                continue
            # distinct failing checks (the loop index in a name is not distinct)
            kinds = {re.sub(r"[0-9]+", "N", l) for l in fails}
            ok = len(kinds) >= 2
            print("  %s  #%d %s -> %d failing check(s), %d distinct"
                  % ("KILLED" if ok else "SURVIVED/WEAK", i, name, len(fails), len(kinds)))
            if verbose:
                for k in sorted(kinds)[:4]:
                    print("          " + k[:150])
            if not ok:
                bad += 1
    if bad:
        print("mutation check FAILED: %d mutant(s) not killed twice" % bad)
        return 1
    print("mutation check: all %d mutants killed by >= 2 distinct checks" % len(MUTANTS))
    return 0


if __name__ == "__main__":
    sys.exit(main())
