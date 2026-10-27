import json
from pathlib import Path
import subprocess

ROOT = Path.cwd()
CASES = [
    ('guard-old-url-secret', 'workflow-guard.js',
     "WORKER_URL: 'vars.VAST_BROKER_URL'", "WORKER_URL: 'secrets.VAST_BROKER_URL'"),
    ('workflow-check-url-secret', '../../.github/workflows/gpu-vast.yml',
     'vars.VAST_BROKER_URL', 'secrets.VAST_BROKER_URL'),
    ('workflow-sweep-url-secret', '../../.github/workflows/gpu-vast-sweep.yml',
     'vars.VAST_BROKER_URL', 'secrets.VAST_BROKER_URL'),
    ('public-url-not-uploaded', 'deploy.mjs',
     'const upload = [...vars];',
     "const upload = [...vars].filter(([key]) => key !== 'WORKER_URL');"),
    ('url-equality-unchecked', 'deploy.mjs',
     "if (vars.has('WORKER_URL') && tomlVar(toml, 'WORKER_URL') !== vars.get('WORKER_URL'))",
     'if (false)'),
    ('toml-origin-unchecked', 'deploy.mjs',
     "if (!bareOrigin(tomlVar(toml, 'WORKER_URL')))", 'if (false)'),
    ('migration-prerequisite-omitted', 'deploy.mjs',
     "'wrangler secret delete WORKER_URL\\n'", "'migration omitted\\n'"),
    ('public-var-allowlist-omitted', 'deploy.mjs', "  'WORKER_URL',\n", ''),
    ('public-toml-origin-omitted', 'wrangler.toml',
     'WORKER_URL = "https://ci.kayfabe.dev"', 'WORKER_URL = "https://other.example"'),
]
for name, relative, old, new in CASES:
    path = ROOT / relative
    original = path.read_text()
    assert original.count(old) == 1, (name, original.count(old))
    try:
        path.write_text(original.replace(old, new))
        run = subprocess.run(
            ['nice', '-n', '19', 'node', '--test', '--test-reporter=tap',
             'test/secret-boundary.test.js', 'test/round-deploy-helper.test.js',
             'test/v5-workflows.test.js', 'test/workflow.test.js'],
            capture_output=True, text=True,
        )
        killers = [line for line in run.stdout.splitlines() if line.startswith('not ok ')]
        print(json.dumps({'mutant': name, 'killers': killers, 'count': len(killers)}), flush=True)
        assert run.returncode and len(killers) >= 2, (name, run.stdout[-3000:], run.stderr)
    finally:
        path.write_text(original)
