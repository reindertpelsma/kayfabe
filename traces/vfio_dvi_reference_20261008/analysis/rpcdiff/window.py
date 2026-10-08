import sys, difflib, collections, json
sys.path.insert(0, '/workspace/nvkvm-rs/.claude/worktrees/agent-a57ba4cb3211566e0/scripts/bench/windows')
import vfio_kf_rpc_diff as T
# window.py GSP_JSONL KF3_LOG START_CLASS END_CLIENT [EXTRA]
# compare from the first alloc of START_CLASS through the last request of client END_CLIENT (+EXTRA requests)
T.MONO_OFFSET = 1791227537632998000
nm = json.load(open('/tmp/claude-0/-workspace-kayfabe/d616bc12-a382-4a26-869c-0f097966b6ab/scratchpad/rpcnames.json'))
T.NAMES = {int(k): v for k, v in nm['fn'].items()}; T.NAMES.update({int(k): v for k, v in nm['event'].items()})
v, _ = T.vfio_stream(sys.argv[1]); k = T.kf_stream(sys.argv[2])
sc, ec = int(sys.argv[3], 16), int(sys.argv[4], 16); extra = int(sys.argv[5]) if len(sys.argv) > 5 else 0
def win(s):
    a = next(i for i, d in enumerate(s) if d['fn'] == 103 and d.get('cls') == sc)
    b = max(i for i, d in enumerate(s) if d.get('client') == ec and d['fn'] == 103)
    return a, min(len(s), b + 1 + extra)
va, vb = win(v); ka, kb = win(k)
V, K = v[va:vb], k[ka:kb]
print('# window VFIO R[%d:%d] %s..%s  kayfabe K[%d:%d] t~%s..%s' % (va, vb, T.utc(V[0]['t']), T.utc(V[-1]['t']), ka, kb, K[0]['near_t'], K[-1]['near_t']))
vk, kk = [T.kind(d) for d in V], [T.kind(d) for d in K]
sm = difflib.SequenceMatcher(None, vk, kk, autojunk=False)
print('## non-equal blocks')
for tag, i1, i2, j1, j2 in sm.get_opcodes():
    if tag == 'equal':
        continue
    print('- %s V[%d:%d] K[%d:%d]: V %s | K %s' % (tag, va + i1, va + i2, ka + j1, ka + j2,
          ' '.join(T.kname(x) + '=' + T.st(d.get('reply')) for x, d in zip(vk[i1:i1 + 8], V[i1:i1 + 8])),
          ' '.join(T.kname(x) + '=' + T.st(d.get('reply')) for x, d in zip(kk[j1:j1 + 8], K[j1:j1 + 8]))))
print('## matched, different status')
for tag, i1, i2, j1, j2 in sm.get_opcodes():
    if tag == 'equal':
        for a, b in zip(V[i1:i2], K[j1:j2]):
            if a.get('reply') != b.get('reply'):
                print('- V %s %s VFIO %s kayfabe %s (client V 0x%x)' % (T.utc(a['t']), T.kname(T.kind(a)), T.st(a.get('reply')), T.st(b.get('reply')), a.get('client', 0)))
cv, ck = collections.Counter(vk), collections.Counter(kk)
print('## only VFIO:', ' '.join('%s x%d' % (T.kname(x), cv[x]) for x in sorted(set(cv) - set(ck))))
print('## only kayfabe:', ' '.join('%s x%d' % (T.kname(x), ck[x]) for x in sorted(set(ck) - set(cv))))
print('## counts differ:', ' '.join('%s %d/%d' % (T.kname(x), cv[x], ck[x]) for x in sorted(set(cv) & set(ck)) if cv[x] != ck[x]))
