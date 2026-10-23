from pathlib import Path
import argparse, subprocess, re, json

B = Path(__file__).resolve().parent
ap = argparse.ArgumentParser()
ap.add_argument("--results", type=Path)
ap.add_argument("--preflight", action="store_true")
args = ap.parse_args()
rows = []
cases = [
    (
        "preauth-before-ledger",
        "src/index.js",
        [("await preauthenticate(request);", "/* mutant enters queue before auth */")],
    ),
    (
        "definitive-refusal-settlement",
        "src/index.js",
        [("if (e instanceof BrokerError && e.state === 'CREATE_REFUSED')", "if (false)")],
    ),
    ("authenticated-admission", "src/index.js", []),
    ("JWKS-cooldown", "src/oidc.js", []),
    (
        "JWKS-single-flight",
        "src/oidc.js",
        [("if (this.refreshing) return this.refreshing;", "if (false) return this.refreshing;")],
    ),
    (
        "id-less-absence",
        "src/vast.js",
        [("if (matches.length === 0) return null;", "if (matches.length === 0) refuse();")],
    ),
    ("spaced-absence-settlement", "src/index.js", []),
    (
        "hold-result-gate",
        "src/index.js",
        [
            (
                "if (r.resultAt === undefined) refuse('UNAUTHORIZED', 401);",
                "/* mutant permits unfinished hold */",
            )
        ],
    ),
    (
        "hold-window-cap",
        "src/index.js",
        [("r.manualDeadline = Math.min(", "r.manualDeadline = Math.max(")],
    ),
    (
        "manual-cancellation",
        "src/index.js",
        [
            (
                "if (record.phase === 'MANUAL' && record.manualOwnerConfirmed) return;",
                "if (record.phase === 'MANUAL') return;",
            )
        ],
    ),
    (
        "front-signature",
        "src/index.js",
        [
            (
                "await (authorize ?? frontAuthorize)(request, env, path);",
                "/* mutant bypasses frontend auth */",
            )
        ],
    ),
    (
        "front-claims",
        "src/index.js",
        [
            (
                "checkedIdentity(await selected.verify(bearer, cfg.audience), cfg);",
                "await selected.verify(bearer, cfg.audience);",
            )
        ],
    ),
    (
        "hashed-deploy",
        "wrangler.toml",
        [
            ('main = "dist/worker.js"', 'main = "src/index.js"'),
            ("no_bundle = true", "no_bundle = false"),
        ],
    ),
    (
        "retry-SHA-release",
        "src/index.js",
        [
            (
                "if (cached?.key === record.key) await ledger.storage.delete('sha:' + record.sha);",
                "/* mutant keeps failed SHA grant */",
            )
        ],
    ),
]
if not args.preflight:
    baseline = subprocess.run(
        [
            "node",
            "--test",
            *[
                str(B / "test" / name)
                for name in [
                    "v5-security-critical.test.js",
                    "v5-create-recovery.test.js",
                    "v5-refused-retry.test.js",
                    "v5-hold-security.test.js",
                    "v5-front-auth.test.js",
                    "v5-deploy-binding.test.js",
                ]
            ],
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert baseline.returncode == 0, "Baseline is not green; refusing mutation attribution."

for name, file, patches in cases:
    p = B / file
    original = p.read_text()
    changed = original
    if name == "authenticated-admission":
        a = changed.index("    if (path === '/availability' || path === '/rent') {")
        b = changed.index("    if (path === '/rent')", a)
        block = changed[a:b]
        changed = changed[:a] + changed[b:]
        at = changed.index("        await preauthenticate(request);")
        changed = (
            changed[:at]
            + "        { const path = await routePath(request,env);\n"
            + block
            + "        }\n"
            + changed[at:]
        )
    elif name == "JWKS-cooldown":
        changed = changed.replace(
            "this.now() - this.lastRefreshAttempt < DEFAULTS.jwksCooldownMs", "false"
        )
    elif name == "spaced-absence-settlement":
        changed = changed.replace("previous !== undefined &&", "false &&")
    else:
        for a, b in patches:
            assert a in changed, (name, a)
            changed = changed.replace(a, b, 1)
    assert changed != original, name
    if args.preflight:
        continue
    try:
        p.write_text(changed)
        assert (
            p.suffix == ".toml"
            or subprocess.run(["node", "--check", str(p)], capture_output=True).returncode == 0
        )
        r = subprocess.run(
            [
                "node",
                "--test",
                str(B / "test/v5-security-critical.test.js"),
                str(B / "test/v5-create-recovery.test.js"),
                str(B / "test/v5-refused-retry.test.js"),
                str(B / "test/v5-hold-security.test.js"),
                str(B / "test/v5-front-auth.test.js"),
                str(B / "test/v5-deploy-binding.test.js"),
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
        killers = re.findall(r"^not ok \d+ - (.+)$", r.stdout, re.M)
        assert r.returncode and len(killers) >= 2, (name, killers, r.stdout[-2000:])
        rows.append(dict(mutant=name, count=len(killers), killers=killers))
        print(name, len(killers), flush=True)
    finally:
        p.write_text(original)
if args.results:
    args.results.write_text(json.dumps(rows, indent=2) + "\n")
if args.preflight:
    print("Security anchors valid:", len(cases))
