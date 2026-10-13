#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation check: break the library on purpose, one way at a time, and require the tests to fail.

A test that still passes with a wrong library is not evidence about that behaviour. Each mutation
is a (file, old text, new text) replacement applied to a scratch copy of lib/; the unit tests are
rebuilt and run; "caught" means a non-zero exit. Survivors are listed, not hidden.
    mutate.py [--out DIR]
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

MUTATIONS = [
    ("walker: a lapped producer is treated as no news", "lib/kfwc_engine.c",
     "e->chain_seq[0] + 1u == s->next_seq", "1"),
    ("walker: seq compare inverted", "lib/kfwc_engine.c",
     "#define SEQ_NEWER(a, b) ((int)((kfwc_u32)(a) - (kfwc_u32)(b)) > 0)",
     "#define SEQ_NEWER(a, b) ((int)((kfwc_u32)(a) - (kfwc_u32)(b)) < 0)"),
    ("walker: ring wrap not split into two records", "lib/kfwc_engine.c",
     "    if (slot + n1 > count)\n        n1 = count - slot;", "    if (0)\n        n1 = count - slot;"),
    ("walker: sink-full loss never announced", "lib/kfwc_engine.c",
     "        if (s->sink_lost &&\n", "        if (0 && s->sink_lost &&\n"),
    ("walker: late attach does not announce the missing prefix", "lib/kfwc_engine.c",
     "        if (info->seq != 0)\n            emit_loss", "        if (0)\n            emit_loss"),
    ("walker: unparseable first-sight history skipped silently", "lib/kfwc_engine.c",
     "if (s->seq_known || wp != 0) {", "if (s->seq_known) {"),
    ("walker: counter assigned after the sink accepted (holes invisible)", "lib/kfwc_engine.c",
     "    r.counter = e->counter++;", "    r.counter = e->counter;"),
    ("walker: detach does not announce the loss", "lib/kfwc_engine.c",
     "    if (reason != KFWC_DETACH_STOPPED)\n        for (d = 0;", "    if (0)\n        for (d = 0;"),
    ("elements: checksum never fails", "lib/kfwc_elem.c",
     "return kfwc_fold32(run, (info->msg_bytes + 7u) & ~7u) == 0;", "return 1;"),
    ("elements: elemCount not cross-checked", "lib/kfwc_elem.c",
     "kfwc_get32(first + l->count_off) != elems", "0"),
    ("discovery: SWAP_RX flag not required", "lib/kfwc_discover.c",
     " ||\n        kfwc_get32(h + KFWC_TXH_FLAGS) != KFWC_MSGQ_FLAG_SWAP_RX)", ")"),
    ("discovery: duplicate region pages accepted", "lib/kfwc_discover.c",
     "            if (b->ptes[j] == v)\n                return 1;", "            if (0)\n                return 1;"),
    ("discovery: page table need not name itself", "lib/kfwc_discover.c",
     "return b->ptes[0] != table_pa;", "return 0;"),
    ("discovery: pages outside RAM accepted", "lib/kfwc_discover.c",
     "if (!v || (v & (KFWC_PAGE - 1)) || !p->ram_contains(p->ctx, v, KFWC_PAGE))",
     "if (!v || (v & (KFWC_PAGE - 1)))"),
    ("platform: register whitelist widened", "lib/kfwc_platform.h",
     "off == KFWC_REG_GSP_MAILBOX1;", "off == KFWC_REG_GSP_MAILBOX1 || off == 0x00110c00u;"),
    ("ring: priority reserve removed", "lib/kfwc_ring.c",
     "kfwc_u32 need = (kind == KFWC_K_ELEMENTS) ? len + r->reserve : len;", "kfwc_u32 need = len;"),
    ("record: LOSS may carry a payload", "lib/kfwc_record.c",
     "    if (r->kind >= KFWC_K_LOSS_OVERFLOW && r->payload_bytes != 0)\n        return 0;", ""),
    ("record: reserved file-header bytes ignored", "lib/kfwc_record.c",
     "    for (i = 37; i < 40; i++)\n        if (b[i])\n            return KFWC_PARSE_BAD;", ""),
]


def build_and_run(libdir, out):
    env = dict(os.environ)
    cflags = ["-std=c99", "-O1", "-g", "-w", "-I" + libdir]
    srcs = [os.path.join(libdir, f) for f in ("kfwc_record.c", "kfwc_elem.c", "kfwc_discover.c", "kfwc_engine.c")]
    for name, extra in (("test_main", srcs), ("test_ring", [os.path.join(libdir, "kfwc_ring.c"), srcs[0]])):
        exe = os.path.join(out, name)
        # tests include "../lib/..." relative to tests/: point them at the scratch copy by copying tests next to it
        r = subprocess.run(["gcc"] + cflags + [os.path.join(os.path.dirname(libdir), "tests", name + ".c")] + extra +
                           ["-lpthread", "-o", exe], capture_output=True, text=True)
        if r.returncode:
            return "build-error"
        r = subprocess.run([exe], capture_output=True, text=True, timeout=300)
        if r.returncode:
            return "caught"
    return "survived"


def main():
    out_root = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else tempfile.mkdtemp(prefix="kfwc-mut-")
    os.makedirs(out_root, exist_ok=True)
    survivors = 0
    base_dir = os.path.join(out_root, "base")
    for d in ("lib", "driver", "tests"):
        shutil.copytree(os.path.join(ROOT, d), os.path.join(base_dir, d), dirs_exist_ok=True)
    if build_and_run(os.path.join(base_dir, "lib"), out_root) != "survived":
        print("the unmutated library does not pass: fix that first")
        return 2
    for name, rel, old, new in MUTATIONS:
        work = os.path.join(out_root, "m")
        shutil.rmtree(work, ignore_errors=True)
        for d in ("lib", "driver", "tests"):
            shutil.copytree(os.path.join(ROOT, d), os.path.join(work, d))
        path = os.path.join(work, rel)
        text = open(path, encoding="utf-8").read()
        if old not in text:
            print("MUTATION TEXT NOT FOUND: %s (%s)" % (name, rel))
            survivors += 1
            continue
        open(path, "w", encoding="utf-8").write(text.replace(old, new, 1))
        res = build_and_run(os.path.join(work, "lib"), out_root)
        print("%-9s %s" % (res, name))
        if res != "caught":
            survivors += 1
    shutil.rmtree(os.path.join(out_root, "m"), ignore_errors=True)
    shutil.rmtree(base_dir, ignore_errors=True)
    print("mutation check: %d mutations, %d not caught" % (len(MUTATIONS), survivors))
    return 1 if survivors else 0


if __name__ == "__main__":
    sys.exit(main())
