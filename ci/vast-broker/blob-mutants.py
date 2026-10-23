"""Blob boundary mutations. Never fetch real blobs or providers."""

import ast, json, re, subprocess
from pathlib import Path

B = Path(__file__).resolve().parent
cases = [
    (
        "capability",
        "src/tokens.js",
        [("  async verify(token, r) {", "  async verify(token, r) { return true;")],
    ),
    (
        "mint-limits",
        "src/blobs.js",
        [("if ((record.blobMints ?? 0) >= runLimit || used >= dayLimit)", "if (false)")],
    ),
    (
        "github-redirect-only",
        "src/blobs.js",
        [("if (response.status !== 302 || response.redirected)", "if (false)")],
    ),
    ("signed-hosts", "src/blobs.js", []),
    (
        "never-binary-body",
        "src/blobs.js",
        [
            (
                "return { url: location, size: asset.size, name };",
                "await response.arrayBuffer(); return { url: location, size: asset.size, name };",
            )
        ],
    ),
    (
        "cancel-binary-body",
        "src/blobs.js",
        [("await response.body?.cancel();", "/* mutant retains stream */")],
    ),
    ("worker-url-redaction", "src/sanitize.js", []),
    ("runner-url-redaction", "runner.py", []),
    (
        "scoped-helper",
        "../../scripts/ci/kf-box-blob-url",
        [("/blob-url/${KF_RUN_ID:?}/", "/blob-url/999-1/")],
    ),
]
rows = []
tests = [str(B / "test/v5-blob.test.js"), str(B / "test/v5-blob-helper.test.js")]
baseline = subprocess.run(["node", "--test", *tests], capture_output=True, text=True)
assert baseline.returncode == 0, "Baseline is not green; refusing attribution"
for name, file, patches in cases:
    p = B / file
    source = p.read_text()
    if name == "signed-hosts":
        a = source.index("![\n")
        z = source.index(".includes(url.hostname)", a) + len(".includes(url.hostname)")
        patches = [(source[a:z], "false")]
    if name == "worker-url-redaction":
        patches = [(next(x for x in source.splitlines() if "// MUTANT signed-url" in x), "")]
    if name == "runner-url-redaction":
        patches = [(next(x for x in source.splitlines() if "# signed-url" in x), "")]
    changed = source
    for before, after in patches:
        assert changed.count(before) == 1, (name, before)
        changed = changed.replace(before, after, 1)
    try:
        p.write_text(changed)
        if p.suffix == ".py":
            ast.parse(changed)
        else:
            assert (
                subprocess.run(
                    [
                        "bash" if (p.suffix == ".sh" or p.name.startswith("kf-")) else "node",
                        "-n" if (p.suffix == ".sh" or p.name.startswith("kf-")) else "--check",
                        str(p),
                    ],
                    capture_output=True,
                ).returncode
                == 0
            ), name
        run = subprocess.run(["node", "--test", *tests], capture_output=True, text=True, timeout=30)
        killers = re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M)
        assert run.returncode and len(killers) >= 2, (name, killers, run.stdout[-2000:])
        rows.append({"mutant": name, "count": len(killers), "killers": killers})
        print(name, len(killers), flush=True)
    finally:
        p.write_text(source)
print(json.dumps(rows, indent=2))
