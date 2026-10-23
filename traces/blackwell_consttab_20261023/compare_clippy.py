from pathlib import Path
import collections
import json
import re

D = Path(__file__).resolve().parent

def diagnostics(path):
    out = {}
    for line in path.read_text().splitlines():
        try:
            j = json.loads(line)
        except json.JSONDecodeError:
            continue
        if j.get('reason') != 'compiler-message':
            continue
        m = j['message']
        package = j['package_id'].split('/crates/')[-1]
        primary = [
            {'file': s['file_name'], 'anchor': s.get('text', [{}])[0].get('text', '')}
            for s in m.get('spans', []) if s.get('is_primary')
        ]
        row = {'package': package, 'level': m['level'],
               'code': (m.get('code') or {}).get('code'), 'message': m['message'],
               'primary': primary}
        out[json.dumps(row, sort_keys=True)] = row
    return out

base = {}
after = {}
for name in ['base_clippy.jsonl', 'base_isolated_clippy.jsonl']:
    base.update(diagnostics(D / name))
for name in ['after_clippy.jsonl', 'after_isolated_clippy.jsonl']:
    after.update(diagnostics(D / name))
added = [after[k] for k in sorted(after.keys() - base.keys())]
removed = [base[k] for k in sorted(base.keys() - after.keys())]
r = {'base_unique_diagnostics': len(base), 'after_unique_diagnostics': len(after),
     'added': added, 'removed': removed,
     'method': 'union of complete isolated RM/display checks and full qemu checks; '
               'deduplicate exact diagnostic+package+primary-source-anchor, retain inherited errors',
     'exit_codes': {name: int((D / (name + '.rc')).read_text()) for name in
                    ['base_clippy', 'base_isolated_clippy', 'after_clippy', 'after_isolated_clippy']}}
(D / 'clippy_compare_complete.json').write_text(json.dumps(r, indent=2) + '\n')
print(json.dumps(r, indent=2))
