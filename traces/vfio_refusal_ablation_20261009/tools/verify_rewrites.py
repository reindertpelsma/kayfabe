#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""verify_rewrites.py GSP_JSONL REFUSE_FILE -- loud, mandatory check that a rewrite actually refused the guest.

Classifies REPLIES only (direction 1); it does not pair a reply back to its own request, because that pairing is
not reliable when the observer's capture has gaps (sequence_gaps in the footer) -- a dropped request/reply shifts
every later FIFO pairing. Instead it uses the fact that the GSP ECHOES the field it was given: for a `field 0`
rule (cmd/hClass rewrite) every reply whose own cmd/hClass equals NEWKEY is a reply to a rewritten request (NEWKEY
is chosen unused, so no other real request collides with it); for a `field obj` rule (hObject rewrite; cmd is left
alone) every reply whose cmd equals the ORIGINAL key is counted directly, since vg_refuse_scan() rewrites every
occurrence of that cmd for as long as the rule is loaded (the whole boot).

GSP_RM_CONTROL (fn 76) reply body: hClient@0 hObject@4 cmd@8 status@12 paramsSize@16 (relative to the body, which
starts 28 bytes after the 'VRPC' marker literal); the VRPC header's rpc_result stays 0 for a control either way --
the real status is the BODY field. GSP_RM_ALLOC (fn 103): hClient@0 hParent@4 hObject@8 hClass@12 status@16; a
failed alloc sets the header's rpc_result AND the body status to the same value.

Prints, per rule: how many replies carry status 0 (REWRITE INEFFECTIVE -- the guest was NOT refused) vs each
non-zero status seen (refused, with the exact value -- compare against kayfabe's own 0x56/0x40 per the brief).
Exit 1 and a loud banner if ANY rule has an ineffective count > 0.
"""
import collections
import json
import struct
import sys


def replies(path):
    """Yield (fn, cmd_or_class_field, status, header_rpc_result) for every fn76/fn103 reply record."""
    for line in open(path, encoding='utf-8-sig'):
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if r.get('kind') != 'record' or r.get('direction') != 1:
            continue
        p = bytes.fromhex(r['payload_hex'])
        i = p.find(b'VRPC')
        if i < 4 or len(p) < i + 28:
            continue
        hv, sig, ln, fn, res, resp, sq, sp = struct.unpack_from('<8I', p, i - 4)
        body = p[i + 28:]
        if fn == 76 and len(body) >= 16:
            cmd, st = struct.unpack_from('<II', body, 8)
            yield 76, cmd, st, res
        elif fn == 103 and len(body) >= 20:
            cls, st = struct.unpack_from('<II', body, 12)
            yield 103, cls, st, res


def load_rules(path):
    """[(fn, key, newkey_or_None, field)]; newkey is None for a default (empty) rewrite -- then this script
    cannot know the exact default key without reimplementing vg_refuse_default_key, so it is computed here."""
    rules = []
    for l in open(path):
        l = l.split('#', 1)[0].split()
        if len(l) < 2:
            continue
        fn, key = int(l[0], 0), int(l[1], 0)
        field = 1 if (len(l) >= 4 and l[3] == 'obj') else 0
        newkey = int(l[2], 0) if len(l) >= 3 and l[2] else default_key(fn, key)
        rules.append((fn, key, newkey, field))
    return rules


def default_key(fn, key):
    """Must track vg_refuse_default_key() in tools/vfio-gsp-observer/core/observer.c exactly."""
    if fn == 76:
        return (key & 0xffff0000) | 0xff00 | (key & 0xff)
    if fn == 103:
        return 0xfff00000 | (key & 0xffff)
    return None


def main():
    gsp, refuse = sys.argv[1], sys.argv[2]
    rules = load_rules(refuse)
    by_echo = collections.defaultdict(list)       # (fn, newkey) -> rule, for field 0
    by_orig = collections.defaultdict(list)        # (fn, key) -> rule, for field 1 (obj)
    for fn, key, newkey, field in rules:
        (by_orig if field else by_echo)[(fn, key if field else newkey)].append((fn, key, newkey, field))
    status = {r: collections.Counter() for r in rules}
    for fn, echoed, st, _ in replies(gsp):
        for r in by_echo.get((fn, echoed), ()):
            status[r][st] += 1
        for r in by_orig.get((fn, echoed), ()):
            status[r][st] += 1
    print('rule                  field  newkey      0(ineffective)  nonzero-status-counts')
    bad = []
    for r in rules:
        fn, key, newkey, field = r
        c = status[r]
        ok0 = c.get(0, 0)
        nz = {hex(k): v for k, v in c.items() if k != 0}
        flag = '  <<< INEFFECTIVE' if ok0 else ''
        print('fn%-4d 0x%08x  %-5s  0x%08x  %-14d  %s%s' %
              (fn, key, 'obj' if field else '-', newkey, ok0, nz, flag))
        if ok0:
            bad.append(r)
    if bad:
        print('=' * 70)
        print('REWRITE INEFFECTIVE: the real GSP returned status 0 (accepted) for a request this')
        print('ablation was supposed to refuse. Rules affected:',
              ', '.join('fn%d/0x%x' % (fn, key) for fn, key, _, _ in bad))
        print('=' * 70)
        sys.exit(1)
    print('all %d rules verified refused (0 ineffective)' % len(rules))


main()
