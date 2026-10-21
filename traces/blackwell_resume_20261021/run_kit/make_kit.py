"""Generate the per-lease kit from the audited 57046541 kit: new source revision, id, branch; no integration check."""
import sys, re, json
from pathlib import Path
sys.path.insert(0, '/workspace/bw-run')
from common_bw import *
iid = int(sys.argv[1]); R = sys.argv[2]; assert len(R) == 40
OLD = Path('/workspace/codex/sweep-win-20261021/57046541'); out = D / str(iid); out.mkdir(exist_ok=True)
AF8 = 'af8bdcda4c700d8b40163ee3bf27feb7cfa6539c'
def adapt(s):
    s = s.replace(AF8, R).replace(AF8[:8], R[:8]).replace('57046541', str(iid))
    s = s.replace('KAYFABE_BRANCH=codex/sweep-win-20261021', 'KAYFABE_BRANCH=claude/blackwell-resume-diag-20261021')
    return s
for f in OLD.iterdir():
    if f.suffix in ('.sh', '.py', '.ps1', '.flags') or f.name == 'common.sh':
        s = adapt(f.read_text())
        if f.name == 'controller.sh':
            keep = []
            for l in s.splitlines():
                if 'deepen=512' in l or 'integration/prod-candidate-5' in l or 'INTEGRATION_ADVANCED_STOP' in l: continue
                keep.append(l)
            s = '\n'.join(keep) + '\n'
            assert 'INTEGRATION' not in s and 'git checkout --detach ' + R in s
        (out / f.name).write_text(s)
lease = next(x for x in json.loads(LEDGER.read_text()) if x['id'] == iid)
(out / 'lease.json').write_text(json.dumps(lease, indent=2) + '\n')
for f in ('arm.sh', 'collect_bw.sh', 'lane.sh'): (out / f).write_text((D / f).read_text())
print(out)
