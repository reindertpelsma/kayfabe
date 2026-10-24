import subprocess, sys, shutil
from pathlib import Path
f = Path("/workspace/bw4-wt/scripts/bench/blackwell/saved_xid32_observation.py")
orig = f.read_text()
M = [
 ("unknown die accepted", 'if type(die) is not str or die not in DIE_PAIRS:', 'if type(die) is not str:'),
 ("no candidate check", 'if authority.get("candidate_die") != die:', 'if False:'),
 ("no authority channel check", 'if authority.get("exact_channels") != list(pair):', 'if False:'),
 ("any channel of any die", 'return die, frozenset(int(channel, 16) for channel in pair)',
  'return die, frozenset(int(c, 16) for p in DIE_PAIRS.values() for c in p)'),
 ("judge skips die", '    die, channels = die_channels(permit, authority)\n    records = []',
  '    die, channels = "GB205", frozenset((0x14, 0x1000000B, 0xE00000B))\n    records = []'),
 ("enable any count", '    if count == 1:\n        return True', '    if count >= 1:\n        return True'),
 ("no label", 'and permit.get("label") == "DIAGNOSTIC"', 'and True'),
 ("ceiling from authority", 'and ceiling == MAX_DIAGNOSTIC_ENABLES', 'and ceiling >= 2'),
 ("count above ceiling", 'and 1 < count <= ceiling', 'and 1 < count'),
 ("bool count", '    if type(count) is not int:\n        return False', '    if not isinstance(count, int):\n        return False'),
 ("scope skips enable check", '    if not enable_count_ok(permit, authority):', '    if False:'),
 ("scope skips die", '    die_channels(permit, authority)\n    if not enable', '    if not enable'),
]
suites = ["test_die_pair_table", "test_saved_xid32_observation", "test_saved_xid32_watch_loop"]
rows=[]
try:
    for name, a, b in M:
        assert orig.count(a) == 1, name
        f.write_text(orig.replace(a, b))
        failed = 0
        for s in suites:
            r = subprocess.run([sys.executable, "-m", "unittest", s], cwd=f.parent, capture_output=True, text=True)
            import re
            m = re.search(r"FAILED \((?:failures=(\d+))?(?:, )?(?:errors=(\d+))?", r.stderr)
            if m: failed += int(m[1] or 0) + int(m[2] or 0)
        rows.append((name, failed))
        print(f"{name:32s} killers={failed}", flush=True)
finally:
    f.write_text(orig)
