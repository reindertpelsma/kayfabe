#!/usr/bin/env python3
"""§39 HARDENING GATES — the guest rewrites GPGA underneath us.

The mapping is LIVE by design (§38): the kernel walks the real RM object, not a
copy.  That makes every load a race with the guest, and makes two shapes fatal:

  (a) DOUBLE FETCH.  Check a value read from GPGA, then read the SAME location
      again and use the second answer.  The load is `volatile`, so the two reads
      can differ and the check governed neither.  Guarded by: exactly one
      dereference site (already gated in check-invariants) PLUS every load site
      naming a DISTINCT offset expression, which is what this script adds.

  (c) CHECK AFTER COALESCE.  The containment test must run before the branch
      that extends a run, or a refused leaf silently grows a legal one past the
      end of the store.
"""
import re, sys

src = open('kf_walk.cu').read()
lines = src.splitlines()
fail = []

# ── (a) every live-GPGA load names a distinct offset ──────────────────────────
def arg2(call):
    depth, start, n = 0, None, 0
    for i, ch in enumerate(call):
        if ch == '(':
            depth += 1
            if depth == 1: start = i + 1
        elif ch == ')':
            depth -= 1
            if depth == 0: break
        elif ch == ',' and depth == 1:
            n += 1
            if n == 1: a = i + 1
            elif n == 2: return call[a:i].strip()
    return None

seen, sites = {}, 0
for ln, line in enumerate(lines, 1):
    if '__forceinline__' in line or line.lstrip().startswith('*'):
        continue
    for m in re.finditer(r'\bkf_(?:load64|win_load)\s*\(', line):
        off = arg2(line[m.end() - 1:])
        if off is None:
            continue
        sites += 1
        key = re.sub(r'\s+', '', off)
        if key in seen:
            fail.append(f"S39(a) FAIL: offset `{off}` is loaded from live GPGA at BOTH "
                        f"line {seen[key]} and line {ln}. Two volatile reads of one "
                        f"location is a DOUBLE FETCH: the guest can change it between "
                        f"them, so whichever check you ran governed neither. Load once "
                        f"into a local and use the copy.")
        seen[key] = ln
if sites < 8:
    fail.append(f"S39(a) FAIL: only {sites} live-GPGA load sites found, want >= 8 -- "
                f"the scan is not matching the code it is supposed to police.")

# ── (c) containment precedes coalescing, in BOTH emit chokepoints ─────────────
for fn in ('kf_emit', 'kf_acc_emit'):
    body, on = [], False
    for ln, line in enumerate(lines, 1):
        if re.match(rf'__device__ __forceinline__ void {fn}\s*\(', line):
            on = True
        if on:
            body.append((ln, line))
            if line.startswith('}'):
                break
    if not body:
        fail.append(f"S39(c) FAIL: {fn} not found -- an emit chokepoint was renamed or removed.")
        continue
    oob = next((ln for ln, l in body if 'KFWR_R_LEAF_OOB' in l), None)
    coal = next((ln for ln, l in body if 'c.run.len += len' in l), None)
    if oob is None:
        fail.append(f"S39(c) FAIL: {fn} emits without a KFWR_R_LEAF_OOB containment check. "
                    f"A leaf pointing outside the store would become a MAPPING.")
    elif coal is not None and oob > coal:
        fail.append(f"S39(c) FAIL: in {fn} the containment check (line {oob}) runs AFTER the "
                    f"coalesce branch (line {coal}). A refused leaf then extends a legal run "
                    f"past the end of the store, with no refusal flag set.")

for f in fail:
    print(f)
if fail:
    sys.exit(1)
print(f"S39(a) ok: {sites} live-GPGA load sites, all distinct offsets -- no double fetch")
print("S39(c) ok: both emit chokepoints bound the leaf BEFORE they coalesce")
