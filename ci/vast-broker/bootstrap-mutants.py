from pathlib import Path
import subprocess, re, json

B = Path(__file__).resolve().parent
rows = []
cases = [
    (
        "required-prefix",
        "src/routes.js",
        "if (env.RENTING_ENABLED === 'true') refuse('PREFIX_REQUIRED', 503);",
        "/* mutant permits missing prefix */",
    ),
    ("create-env", "src/vast.js", "Object.entries(environment).map", "Object.entries({}).map"),
    ("tiny-bootstrap", "src/onstart.js", "exec bash ${DEFAULTS.onstartPath}", "echo skipped"),
]
for name, path, before, after in cases:
    p = B / path
    s = p.read_text()
    assert s.count(before) == 1
    try:
        p.write_text(s.replace(before, after))
        r = subprocess.run(
            ["node", "--test", str(B / "test/v5-auth-bootstrap.test.js")],
            capture_output=True,
            text=True,
        )
        killers = re.findall(r"^not ok \d+ - (.+)$", r.stdout, re.M)
        assert r.returncode and len(killers) >= 2, (name, killers, r.stdout)
        rows.append({"mutant": name, "count": len(killers), "killers": killers})
        print(name, len(killers))
    finally:
        p.write_text(s)
print(json.dumps(rows, indent=2))
