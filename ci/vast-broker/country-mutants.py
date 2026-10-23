"""Country preferences never weaken admission; two named killers per mutation."""

from pathlib import Path
import subprocess, json, re

B = Path(__file__).resolve().parent
p = B / "runner.py"
runner = p.read_text()
start = runner.index('    if "country" in data and (')
end = runner.index('    if data["die"]', start)
guard = runner[start:end]
p = B / "src/policy.js"
policy = p.read_text()
start = policy.index("  if (\n    !preferredCountries.every")
end = policy.index("  const cfg = {", start)
config_guard = policy[start:end]
cases = [
    (
        "country-rank",
        "src/offers.js",
        "          countryRank(a, cfg) - countryRank(b, cfg) ||\n",
        "",
    ),
    (
        "country-config-order",
        "src/offers.js",
        "  const index = cfg.preferredCountries.indexOf(offer.country);",
        "  const index = DEFAULTS.preferredCountries.indexOf(offer.country);",
    ),
    ("country-config-validation", "src/policy.js", config_guard, ""),
    (
        "country-hard-filter",
        "src/offers.js",
        ".filter((x) => order.includes(x.die))",
        ".filter((x) => order.includes(x.die) && x.country === 'US')",
    ),
    ("country-quality-first", "src/offers.js", "if (!reasons.length)", "if (true)"),
    (
        "country-normalization",
        "src/offers.js",
        "return /^[A-Z]{2}$/.test(country) ? country : 'UNKNOWN';",
        "return country;",
    ),
    ("country-ledger", "src/index.js", "        country: offer.country,\n", ""),
    ("country-runner", "runner.py", guard, ""),
    (
        "country-provider-query",
        "src/vast.js",
        "      type: 'ondemand',",
        "      type: 'ondemand', geolocation: {eq:'US'},",
    ),
]
rows = []
for name, file, before, after in cases:
    p = B / file
    source = p.read_text()
    assert source.count(before) == 1, (name, before)
    try:
        p.write_text(source.replace(before, after))
        syntax = subprocess.run(
            (
                ["python3", "-c", "import ast,sys;ast.parse(open(sys.argv[1]).read())", str(p)]
                if p.suffix == ".py"
                else ["node", "--check", str(p)]
            ),
            capture_output=True,
            text=True,
        )
        assert syntax.returncode == 0, (name, syntax.stderr)
        r = subprocess.run(
            ["node", "--test", str(B / "test/v5-country.test.js")], capture_output=True, text=True
        )
        killers = re.findall(r"^not ok \d+ - (.+)$", r.stdout, re.M)
        assert r.returncode and len(killers) >= 2, (name, killers, r.stdout[-1000:])
        rows.append({"mutant": name, "count": len(killers), "killers": killers})
    finally:
        p.write_text(source)
print(json.dumps(rows, indent=2))
