#!/usr/bin/env python3
"""Tabulate llm_parity.sh output: llm_parity_summary.py <file>... (any mix of lanes).

Prints per-lane mean [min..max] (n) for every metric, the guest/<lane> ratios, and text
equality (LLM_TEXT_SHA256 per (kind, ntok)) across lanes."""
import re, sys, statistics as st
from collections import defaultdict

M = defaultdict(list)          # (lane, kind, ntok, metric) -> [values]
SHA = defaultdict(set)         # (kind, ntok) -> {(lane, sha)}
TEXT = {}
for f in sys.argv[1:]:
    for line in open(f, errors='replace'):
        m = re.match(r'.*?LP lane=(\S+) kind=(\S+) ntok=(\d+) proc=(\d+) (.*)$', line.rstrip('\n'))
        if m:
            lane, kind, n, _, rest = m.groups(); n = int(n); k = (lane, kind, n)
            if rest.startswith('LLM_RUN ') or rest.startswith('LLM_GRAPH_RUN '):
                p = rest.split()
                phase = p[2]
                kv = dict(x.split('=', 1) for x in p[3:])
                for key in ('ms', 'tok_s', 'ttft_ms', 'decode_tok_s'):
                    M[k + (phase + '_' + key,)].append(float(kv[key]))
                if kv.get('same_text') == '0':
                    M[k + ('text_drift',)].append(1)
                continue
            kv = rest.split('=', 1)
            if len(kv) != 2: continue
            key, val = kv
            if key == 'LLM_TEXT_SHA256': SHA[(kind, n)].add((lane, val))
            elif key == 'LLM_TEXT': TEXT[(lane, kind, n)] = val
            elif key in ('LLM_MS', 'LLM_TOKENS', 'LLM_T_IMPORT_MS', 'LLM_T_CUINIT_MS',
                         'LLM_T_LOAD_MS', 'LLM_T_TODEV_MS', 'LLM_T_PROC_MS', 'LLM_OK'):
                try: M[k + (key,)].append(float(val))
                except ValueError: pass
            continue
        m = re.match(r'.*?LP_EXT lane=(\S+) kind=(\S+) ntok=(\d+) proc=\d+ ext_ms=(\d+) rc=(\d+)', line)
        if m:
            lane, kind, n, ms, rc = m.groups()
            M[(lane, kind, int(n), 'ext_ms')].append(float(ms))

# derived short tok/s per process
for (lane, kind, n, key), v in list(M.items()):
    if key == 'LLM_MS' and kind == 'short':
        toks = M[(lane, kind, n, 'LLM_TOKENS')]
        M[(lane, kind, n, 'short_tok_s')] = [t * 1000.0 / ms for t, ms in zip(toks, v) if ms > 0]

lanes = sorted({k[0] for k in M}, key=lambda l: (l != 'guest', l))
rows = sorted({(k[1], k[2], k[3]) for k in M})
def fmt(v):
    if not v: return '-'
    mu = st.mean(v)
    return '%.2f [%.2f..%.2f] n=%d' % (mu, min(v), max(v), len(v))
print('| kind | ntok | metric | ' + ' | '.join(lanes) + ' | ' +
      ' | '.join('guest/' + l for l in lanes if l != 'guest') + ' |')
print('|' + '---|' * (3 + len(lanes) + len([l for l in lanes if l != 'guest'])))
for kind, n, key in rows:
    if key in ('LLM_TOKENS', 'LLM_OK'): continue
    vals = [M.get((l, kind, n, key), []) for l in lanes]
    rat = []
    g = M.get(('guest', kind, n, key), [])
    for l in lanes:
        if l == 'guest': continue
        o = M.get((l, kind, n, key), [])
        rat.append('%.3f' % (st.mean(g) / st.mean(o)) if g and o and st.mean(o) else '-')
    print('| %s | %d | %s | %s | %s |' % (kind, n, key, ' | '.join(fmt(v) for v in vals), ' | '.join(rat)))
print()
for (kind, n), s in sorted(SHA.items()):
    shas = {sha for _, sha in s}
    print('TEXT_EQUAL kind=%s ntok=%d lanes=%s equal=%s' % (kind, n, sorted({l for l, _ in s}),
          'YES' if len(shas) == 1 else 'NO (%d distinct)' % len(shas)))
drift = {k: v for k, v in M.items() if k[3] == 'text_drift'}
print('WARM_TEXT_DRIFT=' + (str(sorted(drift)) if drift else 'none'))
for (lane, kind, n), t in sorted(TEXT.items()):
    if kind == 'short': print('TEXT lane=%s ntok=%d: %s' % (lane, n, t[:120]))
