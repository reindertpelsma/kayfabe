"""Focused identity and control-budget mutations; restore every source in finally."""

from pathlib import Path
import subprocess, re, json

B = Path(__file__).resolve().parent
cases = [
    ("repository-id", [("    c.repository_id !== cfg.repositoryId ||\n", "")]),
    (
        "repository-owner-id",
        [
            ("    c.repository_owner_id !== cfg.owner ||\n", ""),
            (
                "    (c.repository_owner_id !== undefined && c.repository_owner_id !== cfg.owner) ||\n",
                "",
            ),
        ],
    ),
    ("environment-subject", [("    c.sub !== cfg.oidcSubject ||\n", "")]),
    (
        "job-workflow-ref",
        [
            (
                "    ![...cfg.checkWorkflows, cfg.sweepWorkflow].includes(c.job_workflow_ref) ||\n",
                "",
            ),
            ("    c.job_workflow_ref !== c.workflow_ref ||\n", ""),
        ],
    ),
    ("control-per-IP", [("count >= cfg.controlIp ||", "false ||")]),
    ("control-global", [("controlCount >= cfg.controlGlobal ||", "false ||")]),
    (
        "configured-base",
        [
            (
                "!cfg.bases.some((base) => c.ref === 'refs/heads/' + base)",
                "c.ref !== 'refs/heads/master'",
            )
        ],
    ),
    (
        "configured-workflow",
        [
            (
                "!cfg.checkWorkflows.includes(c.workflow_ref)",
                "c.workflow_ref !== cfg.repo + '/.github/workflows/gpu-vast.yml@refs/heads/master'",
            )
        ],
    ),
]
rows = []
for name, patches in cases:
    configured = name.startswith("configured-")
    p = B / ("src/github.js" if configured else "src/index.js")
    test = "v5-configured-base.test.js" if configured else "v5-identity-limits.test.js"
    source = p.read_text()
    changed = source
    for before, after in patches:
        assert changed.count(before) == 1, (name, before)
        changed = changed.replace(before, after)
    try:
        p.write_text(changed)
        run = subprocess.run(
            ["node", "--test", str(B / "test" / test)],
            capture_output=True,
            text=True,
        )
        killers = re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M)
        assert run.returncode and len(killers) >= 2, (name, killers, run.stdout)
        rows.append({"mutant": name, "count": len(killers), "killers": killers})
        print(name, len(killers), flush=True)
    finally:
        p.write_text(source)
print(json.dumps(rows, indent=2))
