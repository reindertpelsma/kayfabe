import json
import os
from pathlib import Path
import subprocess

ROOT = Path.cwd()
LEDGER = 'src/ledger.js'
INDEX = 'src/index.js'
RUNNER = 'runner.py'
ORIGINAL = {n: (ROOT / n).read_text() for n in [LEDGER, INDEX, RUNNER]}
CASES = []

def change(name, file, old, new, count=1):
    assert ORIGINAL[file].count(old) == count, (name, ORIGINAL[file].count(old))
    CASES.append((name, {file: ORIGINAL[file].replace(old, new)}))

for reason in ['cost-key', 'sha-gap', 'no-credit', 'daily-count', 'daily-usd', 'rolling-usd',
               'credit-reserve', 'ledger-full', 'invalid-cost', 'credit-unavailable']:
    edits = {}
    for file in [LEDGER, INDEX]:
        old = "budgetRefuse('" + reason + "'"
        if old in ORIGINAL[file]:
            edits[file] = ORIGINAL[file].replace(old, "budgetRefuse('hidden-reason'")
    assert edits, reason
    CASES.append(('diagnostic-' + reason, edits))

old = "const created = record.id ? true : record.closed === 'GONE' ? false : null;"
assert ORIGINAL[LEDGER].count(old) == 2
CASES.append(('no-instance-marker-lost', {LEDGER: ORIGINAL[LEDGER].replace(
    old, 'const created = record.id ? true : null;', 1)}))
change('legacy-no-instance-evidence-ignored', LEDGER,
       "record?.id === null && record.closed === 'GONE' && Number.isFinite(record.ended)", 'false')
change('legacy-interrupted-end-ignored', LEDGER,
       'if (Number.isFinite(record?.ended) && record.started === v.started)', 'if (false)')
start = ORIGINAL[LEDGER].index('  async settle(')
end = ORIGINAL[LEDGER].index('  // Owner ruling 2026-10-24: the log relay', start)
body = ORIGINAL[LEDGER][start:end]
assert body.count('await this.storage.transaction(') == 1
body = body.replace('await this.storage.transaction(', 'await (async (action) => action(this.storage))(')
CASES.append(('settlement-not-transactional', {
    LEDGER: ORIGINAL[LEDGER][:start] + body + ORIGINAL[LEDGER][end:]}))
change('no-instance-still-counted', LEDGER, 'v.started >= day && !uncreated', 'v.started >= day')
change('profiles-mixed', LEDGER, "profile = v.profile ?? 'check'", "profile = 'check'")
change('elapsed-spend-counted-again-as-credit-liability', LEDGER,
       'credit_required_usd: cfg.reserve + cost + summary.outstanding_usd',
       'credit_required_usd: cfg.reserve + cost + summary.rolling_usd')
change('settled-rows-billed-full-ttl', LEDGER, 'const end = v.ended ?? now;',
       'const end = v.ended === null ? now : v.started + v.ttl;')
change('http-reason-omitted', INDEX,
       "            ...(e instanceof BrokerError && e.reason ? { reason: e.reason } : {}),\n", '')
change('http-public-numbers-omitted', INDEX,
       "            ...(e instanceof BrokerError && e.budget ? { budget: e.budget } : {}),\n", '')
change('runner-budget-line-omitted', RUNNER,
       '                    if data["state"] == "UNAVAILABLE_BUDGET":', '                    if False:')
change('runner-unknown-fields-accepted', RUNNER,
       'or not set(values) <= BUDGET_FIELDS', 'or False')
change('admin-prunes-expired-cost-rows', LEDGER, 'excludeKey = null, prune = false',
       'excludeKey = null, prune = true')
change('admin-spend-list-unbounded', LEDGER, 'spend_rows: rows.slice(0, 20)', 'spend_rows: rows')
change('admin-raw-cost-fields-leaked', LEDGER, '        reserved_usd: reserved,',
       '        reserved_usd: reserved, ...v,')
change('admin-outstanding-rows-omitted', LEDGER,
       'outstanding_rows: rows.filter((v) => v.ended === null)', 'outstanding_rows: []')
change('repricing-does-not-exclude-own-reservation', LEDGER,
       "      if (key === 'cost:' + excludeKey) continue;", '')

files = sorted(str(p) for p in Path('test').glob('budget*.test.js'))
files += ['test/v5-offers.test.js', 'workerd/budget.test.js']
env = {**os.environ, 'PYTHONDONTWRITEBYTECODE': '1',
       'PYTHONPYCACHEPREFIX': '/tmp/wk1-budget-disabled-bytecode'}
try:
    for name, edits in CASES:
        for file, source in edits.items(): (ROOT / file).write_text(source)
        run = subprocess.run(['nice', '-n', '19', 'node', '--test', '--test-reporter=tap', *files],
                             capture_output=True, text=True, env=env)
        killers = [line for line in run.stdout.splitlines()
                   if line.startswith('not ok ') or line.lstrip().startswith('not ok ')]
        print(json.dumps({'mutant': name, 'killers': killers, 'count': len(killers)}), flush=True)
        for file in edits: (ROOT / file).write_text(ORIGINAL[file])
        assert run.returncode and len(killers) >= 2, (name, run.stdout[-5000:], run.stderr)
finally:
    for file, source in ORIGINAL.items(): (ROOT / file).write_text(source)
