#!/usr/bin/env python3
"""Offline replay of the `--concurrent-fuzz` POSITIVE CONTROL's decision sequence.

The control (crates/kayfabe-rm-ladder/src/main.rs, `concurrent_fuzz`) runs ONE worker, tid 0,
jitter 0, iters = clamp(fuzz_iters, 24, 96) = 64, seed = cfg.seed ^ 0x385. It passes only if at
least one ENGINE op (write / alias_prop / unmap_b) ran. Before the first engine op no verb draws
from the RNG (jitter is 0; alloc/map_a/map_b/probe/free/recycle take no RNG), so the op sequence
up to the first engine op is a pure function of the seed, assuming every RM verb succeeds
(the control's normal case). This replays W385Rng, W385Op::draw and W385Op::admits exactly.

usage: control_sim.py <seed> ...      replay given seeds (decimal)
       control_sim.py --rate N        estimate P(control draws no engine op) over N random seeds
"""
import random
import sys

M = (1 << 64) - 1
TABLE = (["alloc"] * 3 + ["map_a"] * 3 + ["map_b"] * 3 + ["write"] * 4 + ["alias_prop"] * 3
         + ["unmap_b", "probe", "free", "recycle"])
ENGINE = {"write", "alias_prop", "unmap_b"}
SLOTS = 3


class Rng:
    def __init__(self, s):
        self.s = s & M

    def next(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & M
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M
        return z ^ (z >> 31)

    def below(self, n):
        return 0 if n == 0 else self.next() % n


def admits(op, sl):
    if sl is None:
        return op == "alloc"
    a, b = sl
    return {"alloc": False, "map_a": not a, "map_b": a and not b, "write": a or b,
            "alias_prop": a and b, "unmap_b": b, "probe": a,
            "free": True, "recycle": True}[op]


def control(seed, iters=64):
    """Return (engine_ran, counts) for the control of a run drawn with `seed`."""
    cseed = (seed ^ 0x385) & M
    wseed = (cseed * 0x2545F4914F6CDD1D + 0 * 0x9E3779B97F4A7C15) & M
    rng = Rng(wseed)
    slots = [None] * SLOTS
    counts = {}
    for _ in range(iters):
        op = TABLE[rng.below(len(TABLE))]
        cand = [i for i in range(SLOTS) if admits(op, slots[i])]
        if not cand:
            op, s = "idle", 0
        else:
            s = cand[rng.below(len(cand))]
        counts[op] = counts.get(op, 0) + 1
        if op in ENGINE:
            return True, counts
        if op == "alloc":
            slots[s] = (False, False)
        elif op == "map_a":
            slots[s] = (True, slots[s][1])
        elif op == "map_b":
            slots[s] = (slots[s][0], True)
        elif op == "free":
            slots[s] = None
        elif op == "recycle":
            slots[s] = (False, False)
    return False, counts


def main(argv):
    if argv and argv[0] == "--rate":
        n = int(argv[1])
        r = random.Random(0x385)
        bad = sum(1 for _ in range(n) if not control(r.getrandbits(64) | 1)[0])
        print(f"CONTROL_NO_ENGINE seeds={n} bad={bad} rate={bad / n:.6f}")
        return
    for a in argv:
        ok, c = control(int(a))
        print(f"seed={a} control_engine_ran={ok} counts_until_first_engine_or_end={c}")


if __name__ == "__main__":
    main(sys.argv[1:])
