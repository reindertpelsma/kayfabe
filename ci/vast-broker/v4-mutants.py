#!/usr/bin/env python3
"""Reproducible V4 mutations. No network, credentials, GPU, or npm dependencies.
Run after the legacy campaign; both mutate the same files and must be sequential.
"""
import argparse, json, re, subprocess, sys
from pathlib import Path
from mutation_anchors import apply_mutation

BASE = Path(__file__).resolve().parent
ROOT = BASE.parent.parent
MUTATIONS = [
    (
        "no-result-grace",
        "src/index.js",
        "if(payload.kind==='result'){r.result=",
        "if(payload.kind==='result'){await close(r,vast,'RESULT',payload.result.verdict);r.result=",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "kill-no-destroy",
        "src/index.js",
        "r.boxKilled=true;await close(r,vast,'STOPPED');",
        "r.boxKilled=true;",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "kill-no-capability",
        "src/index.js",
        "r.key!==key||!await tokens.verify(bearer,r)||",
        "r.key!==key||false||",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "kill-wrong-route",
        "../../scripts/ci/kf-box-kill",
        "/kill/${callback.run}",
        "/report/${callback.run}",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "no-absence-verification",
        "src/vast.js",
        "if(await this.inspect(record)!==null)refuse();",
        "/* mutant trusts ACK */",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "no-provision-deadline",
        "src/index.js",
        "record.phase!=='MANUAL'&&record.firstReportAt===undefined&&now()>=record.bootDeadline",
        "false",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "no-cron-reserve",
        "src/policy.js",
        "total:computeStorage*(cfg.hard+cfg.cronPeriod)/3600000+bandwidth",
        "total:computeStorage*cfg.hard/3600000+bandwidth",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "no-hard-ttl",
        "src/index.js",
        "if(record.hardDeadline!==null&&now()>=record.hardDeadline)",
        "if(false)",
        ["v4-lifecycle.test.js", "reporting.test.js", "v5-lifecycle.test.js"],
        "replace",
    ),
    (
        "no-rate-limit",
        "src/index.js",
        "if(r.rateCount>=cfg.reportRate)",
        "if(false)",
        ["v4-lifecycle.test.js", "reporting.test.js"],
        "replace",
    ),
    (
        "address-scrub",
        "src/sanitize.js",
        "'[address]'",
        "'[leaked-address]'",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "addresses",
    ),
    (
        "opaque-token-scrub",
        "src/sanitize.js",
        "value.length>=32?'[redacted]':value",
        "value",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "instance-scrub",
        "src/index.js",
        "bearer,r.id,r.label,r.nonce",
        "bearer,r.nonce",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "full-log-retention",
        "src/index.js",
        "await ledger.appendLog(r,safe.text)",
        "await ledger.appendLog(r,'')",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "rendered-cap",
        "src/index.js",
        "Math.max(0,cfg.reportBytes-(r.fullBytes??0))",
        "4194304",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "summary-verdict",
        "public-log.js",
        "verdict=data.get('verdict','UNAVAILABLE')",
        "verdict='FAIL' if 'FAIL' in data.get('log_tail_sanitised','') else 'PASS'",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "summary-fence",
        "public-log.js",
        "fence=chr(96)*max(3,1+max([len(x) for x in re.findall(chr(96)+r'+',tail)]+[0]))",
        "fence=chr(96)*3",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "renderer-commands",
        "public-log.js",
        "return re.sub(r':{2,}',lambda match:' '.join(match[0]),text).replace('##[','# #[')",
        "return text",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "public-summary-guard",
        "workflow-guard.js",
        'if(/GITHUB_ENV/.test(line)||/GITHUB_STEP_SUMMARY/.test(line)&&!(approvedRenderer&&line.trim()==="with '
        "open(os.environ['GITHUB_STEP_SUMMARY'],'a',encoding='utf-8') as summary:\"))",
        "if(false)",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "workflow-fail-grace",
        "../../.github/workflows/gpu-vast.yml",
        '&& "$terminal_result" != true',
        "&& true",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "workflow-cursor",
        "../../.github/workflows/gpu-vast.yml",
        "            fetch_new_logs",
        "            cursor=0\n            fetch_new_logs",
        ["v4-public-log.test.js", "client.test.js", "secret-boundary.test.js"],
        "replace",
    ),
    (
        "sender-discards-prefix",
        "../../scripts/ci/vast_report.py",
        "pending+=data",
        "pending=(pending+data)[-8192:]",
        ["v4-sender.test.js"],
        "old-buffer",
    ),
    (
        "sender-repeats-tail",
        "../../scripts/ci/vast_report.py",
        "'Suite complete; sender omitted '+str(omitted)+' raw bytes beyond its bounded transport allowance.'",
        "tail",
        ["v4-sender.test.js"],
        "replace",
    ),
    (
        "unreleased-upload",
        "src/index.js",
        "await releaseUpload(request);busy=false",
        "busy=false",
        ["v4-request-body.test.js"],
        "replace",
    ),
    (
        "direct-network-body",
        "src/index.js",
        "buffered=await detachedUpload(request,path)",
        "buffered=request",
        ["v4-ingress.test.js"],
        "replace",
    ),
    (
        "unbounded-forward-body",
        "src/index.js",
        "const kept=value.slice(0,limit-length)",
        "const kept=value",
        ["v4-ingress.test.js"],
        "replace",
    ),
    (
        "remaining-grace-ttl-cap",
        "src/index.js",
        "nextDeadline(r)-now()",
        "r.resultDeadline-now()",
        ["v4-result-ack.test.js", "v4-sender.test.js"],
        "replace",
    ),
    (
        "remaining-grace-ack",
        "src/index.js",
        "...(payload.kind==='result'?{grace_remaining_seconds:",
        "...(false?{grace_remaining_seconds:",
        ["v4-result-ack.test.js", "v4-sender.test.js"],
        "replace",
    ),
    (
        "sender-ignores-grace-ack",
        "../../scripts/ci/vast_report.py",
        "seconds=ack.get('grace_remaining_seconds') if isinstance(ack,dict) else None",
        "seconds=None",
        ["v4-result-ack.test.js", "v4-sender.test.js"],
        "replace",
    ),
    (
        "lose-result-after-box-kill",
        "src/index.js",
        "r.verdict||'UNAVAILABLE':'UNAVAILABLE'",
        "(r.boxKilled?'UNAVAILABLE':r.verdict)||'UNAVAILABLE':'UNAVAILABLE'",
        ["v4-kill-result.test.js"],
        "replace",
    ),
    (
        "stale-status-cursor",
        "../../.github/workflows/gpu-vast.yml",
        '"$next" -gt "$cursor" && "$next" -le 10000',
        '"$next" -gt "$cursor" && "$next" -le "$target"',
        ["client.test.js"],
        "replace",
    ),
    (
        "overlapping-worker-delimiters",
        "src/sanitize.js",
        "replace(/:{2,}/g,value=>value.split('').join(' '))",
        "replaceAll('::',': :')",
        ["v4-delimiters.test.js"],
        "replace",
    ),
    (
        "overlapping-renderer-delimiters",
        "public-log.js",
        "re.sub(r':{2,}',lambda match:' '.join(match[0]),text)",
        "text.replace('::',': :')",
        ["v4-delimiters.test.js"],
        "replace",
    ),
]


def changed(source, before, after, kind):
    if kind == "addresses":
        assert source.count("'[address]'") == 2
        return "\n".join(line for line in source.splitlines() if "'[address]'" not in line) + "\n"
    assert source.count(before) == 1, "mutation anchor missing or ambiguous"
    result = source.replace(before, after)
    if kind == "old-buffer":
        result = result.replace(
            "if pending and (len(pending)>=8192 or current-last>=1):",
            "if pending and current-last>=1:",
        )
    return result


# Inline shell/renderer guard probes are replaced by the BASE runner campaign.
MUTATIONS = [
    m
    for m in MUTATIONS
    if m[0]
    not in {"public-summary-guard", "workflow-fail-grace", "workflow-cursor", "stale-status-cursor"}
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--results", type=Path)
    parser.add_argument("--filter", default="")
    args = parser.parse_args()
    rows = []
    for name, file, before, after, tests, kind in MUTATIONS:
        if args.filter and args.filter not in name:
            continue
        path = BASE / file
        original = path.read_text()
        mutated = apply_mutation("v4", name, original)
        assert mutated != original, name
        if args.preflight:
            continue
        try:
            path.write_text(mutated)
            if file.endswith(".yml"):
                run = (
                    mutated.split("  gpu:\n")[1]
                    .split("  report:\n")[0]
                    .split("        run: |\n")[1]
                )
                run = "\n".join(
                    line[10:] if line.startswith("          ") else line
                    for line in run.splitlines()
                )
                syntax = subprocess.run(["bash", "-n"], input=run, text=True, capture_output=True)
            else:
                syntax = subprocess.run(
                    (
                        [
                            "python3",
                            "-c",
                            "import ast,sys;ast.parse(open(sys.argv[1]).read())",
                            str(path),
                        ]
                        if path.suffix == ".py"
                        else (
                            ["bash", "-n", str(path)]
                            if (path.suffix == ".sh" or path.name.startswith("kf-"))
                            else ["node", "--check", str(path)]
                        )
                    ),
                    text=True,
                    capture_output=True,
                )
            assert syntax.returncode == 0, name + ": invalid mutant syntax"
            result = subprocess.run(
                ["node", "--test", *[str(BASE / "test" / test) for test in tests]],
                cwd=ROOT,
                text=True,
                capture_output=True,
                timeout=60,
            )
            killers = re.findall(r"^not ok \d+ - (.*)", result.stdout, re.M)
            assert result.returncode != 0 and len(killers) >= 2, (
                name + ": insufficient independently named killers (" + str(len(killers)) + ")"
            )
            rows.append({"mutant": name, "killers": len(killers), "tests": killers})
            print(name + ": " + str(len(killers)) + " tests failed", flush=True)
        finally:
            path.write_text(original)
    if args.results and not args.preflight:
        args.results.write_text(json.dumps(rows, indent=2) + "\n")
    print(
        ("All anchors valid: " if args.preflight else "All mutants killed and sources restored: ")
        + str(len(MUTATIONS))
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
