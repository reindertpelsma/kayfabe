#!/usr/bin/env python3
"""
nvd_progress.py — the PROGRESS FRACTION, derived rather than eyeballed.

The differential's headline number is *how far into `cuCtxCreate` the guest gets
before it stops matching hardware*: 221 of 479 (46.1%) at the first firing.  That
number was read off a `diff` listing by hand, and a number read by hand is a
number that moves when the reader does.  This computes it from the captures.

    usage: nvd_progress.py <host_ctx.jsonl[.zst]> <guest_ctx.jsonl[.zst]>
                           <host_dev.jsonl[.zst]> <guest_dev.jsonl[.zst]>

Definition, stated so it can be argued with rather than inferred:

  * `cuCtxCreate`'s ioctls are the records a `ctx` capture has and the `dev`
    capture of the SAME subject does not.  Each stage is a strict prefix of the
    next (see the reference MANIFEST), so the first `cuCtxCreate` ioctl is at
    index `len(dev)` on each side.  On the host that is 129 and the stage adds
    479 records; those 479 are the denominator.
  * The two streams are aligned by opcode with the SAME `difflib` matcher `diff`
    uses — never a second alignment, or the fraction and the divergence listing
    could disagree.
  * ⊘⊘ The device-node NAME is canonicalised out of the opcode first, and this is
    not a convenience.  The reference host is a FIVE-GPU rig whose chosen GPU is
    minor **7**, so `nvidia7:REGISTER_FD` and `nvidia0:REGISTER_FD` are the same
    call spelled differently and `difflib` reports the block as `replace`.
    `[measured 2026-08-10]` an uncanonicalised run of this script stops there and
    reports **131/479 (27.3%)** for a capture whose real answer is 221 — i.e. the
    rig's minor number, read as a wall.  Rank by kind, never by index.
  * Progress ends at the first divergence where the GUEST emits calls hardware
    does not (`insert`/`replace`) at or after `cuCtxCreate`'s first ioctl.
    ⊘ Host-only runs (`delete`) do NOT end it: on a five-GPU reference those are
    the four GPUs we do not have, so ending there would measure the rented box.
    They are counted and printed instead.
  * The count is the number of GUEST records inside `equal` blocks at index
    >= `len(guest_dev)`, before that stop — not an index range, because a skipped
    host-only run makes an index range and a count different numbers.
    ⊘ Deliberately structural-only: a STATUS or VALUE divergence is a wrong
    answer to a question the guest still asked and still got past, and counting
    it as the end would make "fixed a status" and "advanced a rung" the same
    number.  They are not, and this whole instrument exists to tell them apart.

⊘ WHAT THIS NUMBER IS NOT.  It is an ioctl-boundary, control-plane measure.  It
cannot see the doorbell, pushbuffer, semaphore or completion planes, and the
reference's own `ce`/`launch` stages are byte-identical to `ctx`, so a kernel
launch costs zero ioctls and would move this number by zero.  A rung that moves
it is real; a rung that does not is not thereby absent.
"""

import sys
import difflib

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from nvdiff import load  # noqa: E402


NODE = __import__("re").compile(r"^nvidia\d+:")


def canon(op):
    """`nvidia7:REGISTER_FD` and `nvidia0:REGISTER_FD` are one call.  See the docstring."""
    return NODE.sub("nvidia*:", op)


def progress(host_ctx, guest_ctx, host_dev, guest_dev):
    A, B = load(host_ctx), load(guest_ctx)
    na, nb = len(load(host_dev)), len(load(guest_dev))
    total = len(A) - na
    sm = difflib.SequenceMatcher(a=[canon(r["op"]) for r in A],
                                 b=[canon(r["op"]) for r in B], autojunk=False)
    done = 0
    host_only = 0
    stop = None
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            done += max(0, j2 - max(j1, nb)) if i2 > na else 0
            continue
        if tag == "delete":
            # Host-only calls: on the five-GPU reference these are the GPUs we do
            # not have.  Counted, never a wall.
            if i1 >= na:
                host_only += i2 - i1
            continue
        if i1 >= na or j1 >= nb:
            stop = (tag, i1, j1, A[i1]["op"] if i1 < len(A) else "-",
                    B[j1]["op"] if j1 < len(B) else "-", j2 - j1)
            break
    print("host   dev=%d  ctx=%d   -> cuCtxCreate is %d ioctls" % (na, len(A), total))
    print("guest  dev=%d  ctx=%d" % (nb, len(B)))
    print("host-only calls skipped as environmental (the reference rig's other GPUs): %d"
          % host_only)
    if stop:
        tag, i1, j1, oa, ob, n = stop
        print("WALL: guest emits %d call(s) hardware does not — %s at A[%d]=%s  B[%d]=%s"
              % (n, tag, i1, oa, j1, ob))
    else:
        print("no guest-side EXTRA divergence in the aligned region")
    print("guest completes %d of cuCtxCreate's %d ioctls" % (done, total))
    print("PROGRESS %d/%d = %.1f%%" % (done, total, 100.0 * done / total))
    return done, total


if __name__ == "__main__":
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    progress(*sys.argv[1:5])
