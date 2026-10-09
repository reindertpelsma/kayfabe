#!/usr/bin/env python3
"""Delta-debugging search over the refusal list: which refusals make the real driver fail?

STATUS: LIVE, 2026-10-09. Tool only: the oracle (one VFIO boot) is a command the caller supplies.

Units are components (one ogkm ctrl header, or one class family for allocs), not raw control ids.
Stage 1 minimises over components, stage 2 over the keys inside the components that stage 1 kept.
ddmin (Zeller) finds a 1-minimal failing subset, so an interaction (X fails only when Y is also
refused) shows up as a minimal set of size >= 2. After each culprit set is found it is removed
from the full list and the search repeats on the rest, so every independent cause is found.

  ddsearch.py --list refusal-full.txt --ogkm DIR --oracle './boot_once.sh {subset} {out}' --work W

Oracle contract: runs one boot with the refusal file {subset}, writes evidence to {out}, and exits
0 = driver healthy, 1 = driver dead (Code 43 / nvidia-smi fails 3 min after LogonUI),
2 = invalid run (no boot, host problem). Results are cached in W/cache.json by subset content.
Every distinct subset is run --reps times (default 2); disagreement runs a third time and the
majority wins; a 1-1-1 outcome aborts the search (flaky, report it, do not guess).
"""
import argparse, hashlib, json, os, re, subprocess, sys

PASS, FAIL = "pass", "fail"


def parse_list(path):
    head, rules = [], []
    for ln in open(path):
        s = ln.rstrip("\n")
        if not s.strip() or s.lstrip().startswith("#"):
            head.append(s)
            continue
        f = s.split("#")[0].split()
        rules.append((int(f[0]), int(f[1], 0), s))
    return head, rules


def component_map(ogkm):
    """cmd value -> header stem, from ogkm's ctrl headers (nothing else is consulted)."""
    m = {}
    if not ogkm:
        return m
    base = os.path.join(ogkm, "src")
    pat = re.compile(r"#define\s+NV\w*_CTRL_CMD\w*\s+\(?\s*(0x[0-9a-fA-F]+)\s*\)?")
    for root, _, files in os.walk(base):
        for fn in files:
            if fn.endswith(".h") and fn.startswith("ctrl"):
                for ln in open(os.path.join(root, fn), errors="replace"):
                    g = pat.match(ln)
                    if g:
                        v = int(g.group(1), 16)
                        m.setdefault(v, fn[:-2])
                        m.setdefault(v >> 8, fn[:-2])
    return m


def component(rule, cmap):
    fn, key, _ = rule
    if fn == 103:
        return "alloc"
    if key in cmap:
        return cmap[key]
    if (key >> 8) in cmap:          # same ogkm category byte as a named control
        return cmap[key >> 8]
    return "unnamed_%04x" % (key >> 16) + "_%02x" % ((key >> 8) & 0xFF)


class Oracle:
    def __init__(self, template, work, head, reps):
        self.t, self.w, self.head, self.reps = template, work, head, reps
        self.cp = os.path.join(work, "cache.json")
        self.cache = json.load(open(self.cp)) if os.path.exists(self.cp) else {}
        self.runs = 0

    def _boot(self, rules, tag):
        body = "\n".join(r[2] for r in rules)
        path = os.path.join(self.w, "subset-%s.txt" % tag)
        open(path, "w").write("\n".join(self.head) + "\n" + body + "\n")
        out = os.path.join(self.w, "out-%s" % tag)
        os.makedirs(out, exist_ok=True)
        cmd = self.t.format(subset=path, out=out)
        for _ in range(3):
            rc = subprocess.call(cmd, shell=True)
            self.runs += 1
            if rc in (0, 1):
                return PASS if rc == 0 else FAIL
            print("  invalid run (rc=%d), retrying" % rc, file=sys.stderr)
        sys.exit("oracle keeps returning invalid; fix the host first")

    def test(self, rules):
        key = hashlib.sha1("\n".join(sorted(r[2] for r in rules)).encode()).hexdigest()[:12]
        if key in self.cache:
            return self.cache[key]
        votes = [self._boot(rules, "%s-a" % key) for _ in range(1)]
        for i in range(1, self.reps):
            votes.append(self._boot(rules, "%s-%s" % (key, "abc"[i])))
        if len(set(votes)) > 1:
            votes.append(self._boot(rules, "%s-c" % key))
            if sorted(votes).count(votes[0]) < 2 and len(set(votes)) == 3:
                sys.exit("flaky oracle on %s: %s" % (key, votes))
        res = max(set(votes), key=votes.count)
        self.cache[key] = res
        json.dump(self.cache, open(self.cp, "w"), indent=1)
        print("  %d rules -> %s %s" % (len(rules), res, votes))
        return res


def ddmin(units, fails):
    """units: list; fails(subset)->bool. Returns a 1-minimal failing subset (fails(units) must hold)."""
    n = 2
    while len(units) >= 2:
        size = max(1, len(units) // n)
        chunks = [units[i:i + size] for i in range(0, len(units), size)]
        reduced = False
        for c in chunks:                       # reduce to a failing chunk
            if len(c) < len(units) and fails(c):
                units, n, reduced = c, 2, True
                break
        if not reduced:
            for i in range(len(chunks)):       # reduce to a failing complement
                comp = [u for j, c in enumerate(chunks) if j != i for u in c]
                if comp and fails(comp):
                    units, n, reduced = comp, max(n - 1, 2), True
                    break
        if not reduced:
            if n >= len(units):
                break
            n = min(len(units), n * 2)
    return units


def search(rules, cmap, oracle):
    rest = list(rules)
    found = []
    while rest:
        if oracle.test(rest) != FAIL:
            break
        comps = sorted({component(r, cmap) for r in rest})
        by = lambda cs: [r for r in rest if component(r, cmap) in set(cs)]
        fails_c = lambda cs: oracle.test(by(cs)) == FAIL
        cs = ddmin(comps, fails_c) if len(comps) > 1 else comps
        keys = by(cs)
        ks = ddmin(keys, lambda s: oracle.test(s) == FAIL) if len(keys) > 1 else keys
        print("CULPRIT SET (components %s): %s" % (cs, [hex(r[1]) for r in ks]))
        found.append({"components": cs, "rules": [[r[0], hex(r[1])] for r in ks]})
        rest = [r for r in rest if r not in ks]
    return found, rest


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", required=True)
    ap.add_argument("--ogkm")
    ap.add_argument("--oracle", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--reps", type=int, default=2)
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    head, rules = parse_list(a.list)
    cmap = component_map(a.ogkm)
    orc = Oracle(a.oracle, a.work, head, a.reps)
    if orc.test([]) != PASS:
        sys.exit("baseline (no refusals) does not pass: the oracle or the host is broken")
    found, rest = search(rules, cmap, orc)
    res = {"culprit_sets": found, "remaining_rules_not_failing": len(rest), "boots": orc.runs}
    json.dump(res, open(os.path.join(a.work, "result.json"), "w"), indent=1)
    print(json.dumps(res, indent=1))


if __name__ == "__main__":
    main()
