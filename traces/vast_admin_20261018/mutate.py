"""UI1 local-only mutation probes; restore each source before running the next mutant."""

from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
MAIN = "ci/vast-broker/src/index.js"
ADMIN = "ci/vast-admin/worker.js"
ROUTES = "ci/vast-broker/src/routes.js"
VAST = "ci/vast-broker/src/vast.js"
TESTS = ["ci/vast-broker/test/admin.test.js", "ci/vast-admin/test/worker.test.mjs"]

# FALSIFIER: a mutant survives, fails fewer than two independent tests, or cannot be restored.
MUTANTS = [
    (
        "M23 JWT signature verification bypass",
        ADMIN,
        "const verified = await jwtVerify(assertion, keys, {\n"
        "        algorithms: ['RS256'],\n"
        "        issuer,\n"
        "        audience: env.ACCESS_AUDIENCE,\n"
        "        currentDate: new Date(now()),\n"
        "        requiredClaims: ['exp', 'iat'],\n"
        "      });",
        "const verified = {payload: JSON.parse(atob(assertion.split('.')[1]))};",
    ),
    (
        "M24 truncated ledger accepted",
        MAIN,
        "if (entries.size >= 1000) return json({ error: 'Ledger limit reached' }, 503);",
        "",
    ),
    (
        "M01 internal origin bypass",
        MAIN,
        "url.origin !== 'https://internal.invalid' || url.search",
        "url.search",
    ),
    (
        "M02 public routing exposed",
        ROUTES,
        "export function routeKind(path) {",
        "export function routeKind(path) {\n  if (path.startsWith('/admin/')) return 'runner';",
    ),
    (
        "M03 raw record disclosure",
        MAIN,
        "run: text(r.key),",
        "...r,\n                run: text(r.key),",
    ),
    (
        "M04 scalar whitelist bypass",
        MAIN,
        "(typeof v === 'string' ? v.slice(0, 256) : null)",
        "(v ?? null)",
    ),
    ("M05 oldest first", MAIN, "b.started - a.started", "a.started - b.started"),
    ("M06 output cap removed", MAIN, ".slice(0, 100)", ".slice(0, 1000)"),
    (
        "M07 ambiguous selection accepted in DO",
        MAIN,
        "Object.keys(selection).length !== 1",
        "false",
    ),
    (
        "M08 stop bypasses owner service",
        MAIN,
        "const response = await this.service.fetch(",
        "return json({ state: 'NONE' });\n        const response = await this.service.fetch(",
    ),
    ("M09 failed cleanup reported as success", MAIN, "return response.ok", "return true"),
    (
        "M10 owner authority omitted",
        MAIN,
        "headers: { Authorization: 'Bearer ' + this.env.OWNER_STOP_TOKEN }",
        "headers: {}",
    ),
    (
        "M11 deletion acknowledgement treated as absence",
        VAST,
        "if ((await this.inspect(record)) !== null) refuse();",
        "await this.inspect(record);",
    ),
    ("M12 owner check bypass", ADMIN, "identity.email !== env.OWNER_EMAIL", "false"),
    ("M13 issuer check omitted", ADMIN, "issuer,\n        audience:", "audience:"),
    ("M14 audience check omitted", ADMIN, "audience: env.ACCESS_AUDIENCE,", ""),
    (
        "M15 time validation weakened",
        ADMIN,
        "currentDate: new Date(now()),",
        "currentDate: new Date(now()), clockTolerance: 120,",
    ),
    ("M16 empty audience not fail-closed", ADMIN, "!env.ACCESS_AUDIENCE.trim()", "false"),
    (
        "M17 ambiguous selection accepted in UI",
        ADMIN,
        "Object.keys(selection).length !== 1",
        "false",
    ),
    ("M18 proxy paths swapped", ADMIN, "(list ? 'list' : 'stop')", "(list ? 'stop' : 'list')"),
    (
        "M19 duplicate stop proxy call",
        ADMIN,
        "if (!response.ok) return json({ error: 'Broker unavailable' }, response.status);",
        "if (stop) await stub.fetch(new Request('https://internal.invalid/admin/stop'));\n"
        "      if (!response.ok) return json({ error: 'Broker unavailable' }, response.status);",
    ),
    (
        "M20 Stop all confirmation bypass",
        ADMIN,
        "if (!confirm('Stop all active runs?')) return;",
        "",
    ),
    ("M21 ended run gets Stop button", ADMIN, "if (run.ended_at === null)", "if (true)"),
    (
        "M22 untrusted cells interpreted as HTML",
        ADMIN,
        "td.textContent = value == null",
        "td.innerHTML = value == null",
    ),
]


def probe():
    originals = {name: (ROOT / name).read_text() for _, name, _, _ in MUTANTS}
    results = []
    try:
        for label, name, before, after in MUTANTS:
            original = originals[name]
            if original.count(before) != 1:
                raise RuntimeError(f"mutation anchor not unique: {label}")
            path = ROOT / name
            try:
                path.write_text(original.replace(before, after))
                run = subprocess.run(
                    ["node", "--test", *TESTS],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    timeout=45,
                    check=False,
                )
            finally:
                path.write_text(original)
            kills = re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M)
            if run.returncode == 0 or len(kills) < 2:
                raise RuntimeError(f"{label}: {len(kills)} kills; expected at least two")
            results.append((label, kills))
            print(f"{label}: {len(kills)} killing tests", flush=True)
    finally:
        for name, original in originals.items():
            (ROOT / name).write_text(original)
    lines = ["| Mutant | Killing tests | Two independent witnesses |", "|---|---:|---|"]
    for label, kills in results:
        lines.append(f"| {label} | {len(kills)} | {'; '.join(kills[:2])} |")
    (Path(__file__).parent / "mutations.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    probe()
