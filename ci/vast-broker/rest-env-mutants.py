"""REST create environment mutations; require two named contract killers each."""

from pathlib import Path
import json
import re
import subprocess

BASE = Path(__file__).resolve().parent
path = BASE / "src/vast.js"
source = path.read_text()
block = source[
    source.index("      env: Object.fromEntries(") : source.index("      label: record.label,")
]
cases = [
    (
        "REST-env-as-text",
        block,
        block.replace(
            "env: Object.fromEntries(", "env: JSON.stringify(Object.fromEntries("
        ).replace("      ),\n", "      )),\n"),
    ),
    (
        "REST-protocol-missing",
        "return [key, String(value)];",
        "return [key === 'KF_PROTOCOL' ? 'KF_BAD_PROTOCOL' : key, String(value)];",
    ),
    (
        "REST-runner-nonce-missing",
        "return [key, String(value)];",
        "return [key === 'KF_RUNNER_NONCE' ? 'KF_BAD_NONCE' : key, String(value)];",
    ),
]
rows = []
for name, before, after in cases:
    assert source.count(before) == 1
    try:
        path.write_text(source.replace(before, after))
        syntax = subprocess.run(["node", "--check", str(path)], capture_output=True, text=True)
        assert syntax.returncode == 0, syntax.stderr
        result = subprocess.run(
            ["node", "--test", str(BASE / "test/v5-rest-env.test.js")],
            capture_output=True,
            text=True,
        )
        killers = re.findall(r"^not ok \d+ - (.+)$", result.stdout, re.M)
        assert result.returncode and len(killers) >= 2, (name, killers, result.stdout)
        rows.append({"mutant": name, "count": len(killers), "killers": killers})
    finally:
        path.write_text(source)
print(json.dumps(rows, indent=2))
