"""Public URLs, compact stage lines and the unbudgeted overview/verdict of the runner (2026-11-12).

usage: runner_urls_fixture.py <case>      (KF_RUNNER_PATH overrides the runner under test)
Falsifiers: LOG_URL/STATUS_URL absent before the first Worker call, a URL built from an id that is
not the validated run id and leg, the verdict or the URLs lost once the display budget is spent.
"""

import io, os, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import runner_head_tail_fixture as hf  # loads the runner under test (KF_RUNNER_PATH)

m = hf.m
RUN = "123-1"
LOG0 = "https://logs.kayfabe.dev/vast-123-1-0/log.txt"
STATUS0 = "https://kayfabe.dev/status/run/?id=vast-123-1-0"


def case_start():
    """Both URLs are printed before the first Worker call (before any box output exists)."""
    printed, seen = [], []

    def client(route, method="GET", body=None):
        seen.append("".join(printed))
        raise OSError("stop")

    m.Runner(hf.ctx, hf.manifest, client, emit=printed.append, summary=io.StringIO()).execute()
    first = seen[0]
    assert "LOG_URL leg=0 " + LOG0 + "\n" in first, first
    assert "STATUS_URL leg=0 " + STATUS0 + "\n" in first, first


def case_start_order():
    """The two lines come in that order, each on a line of its own."""
    printed = []
    r = m.Runner(hf.ctx, hf.manifest, lambda *_: None, emit=printed.append)
    r.announce(0)
    r.announce(0)  # once only
    out = "".join(printed)
    assert out == "LOG_URL leg=0 %s\nSTATUS_URL leg=0 %s\n" % (LOG0, STATUS0), out


def case_urls_exact():
    assert m.public_urls(RUN, 0) == (LOG0, STATUS0)
    assert (
        m.public_urls("43092074455-1", 3)[1]
        == "https://kayfabe.dev/status/run/?id=vast-43092074455-1-3"
    )


def case_bad_run():
    """An id that is not `<digits>-<digits>` never reaches a URL (injection, path, scheme)."""
    for bad in [
        "123-1/../../x",
        "123-1\n::set-output name=x::y",
        "123-1?x=1",
        "evil.example/123-1",
        "123-1 ",
        "0123-1",
        "123",
        "",
        None,
        123,
        "1" * 21 + "-1",
        "123-1#frag",
        "https://evil.example/a-1",
    ]:
        assert m.public_urls(bad, 0) is None, bad
    printed = []
    r = m.Runner(dict(hf.ctx, run="123-1/../x"), hf.manifest, lambda *_: None, emit=printed.append)
    r.announce(0)
    assert "".join(printed) == "", printed


def case_bad_leg():
    """The leg comes from a Worker integer: bools, negatives, big numbers and strings give none."""
    for bad in [True, False, -1, 32, 10**6, "0", "1; rm", None, 1.0, [0]]:
        assert m.public_urls(RUN, bad) is None, bad
    assert m.public_urls(RUN, 31) is not None
    printed = []
    r = m.Runner(hf.ctx, hf.manifest, lambda *_: None, emit=printed.append)
    r.announce(1)
    assert "vast-123-1-1/log.txt" in "".join(printed)


def case_bases_pinned():
    assert m.LOGS_BASE == "https://logs.kayfabe.dev" and not m.LOGS_BASE.endswith("/")
    assert m.STATUS_BASE == "https://kayfabe.dev/status/run/?id="


def case_verdict_under_budget():
    """A log that spends the whole display budget still ends with the verdict and both URLs again."""
    out, result = hf.run("".join(hf.row(i) for i in range(1500)))
    lines = out.rstrip().splitlines()
    assert lines[-1] == "VERDICT FAIL state=GRACE verified=1", lines[-3:]
    assert lines[-2] == "::endgroup::"
    assert re.search(r"^\[\d+ bytes dropped by runner display budget\]$", out, re.M)
    assert out.count("LOG_URL leg=0 " + LOG0) >= 2, "start and overview"
    assert out.count("STATUS_URL leg=0 " + STATUS0) >= 2
    assert re.search(r"^OVERVIEW tested_sha=" + "a" * 40, out, re.M)


def case_verdict_budget_spent_first():
    """Even when show() has been refused for a long time, direct lines are never dropped."""
    printed = []
    r = m.Runner(hf.ctx, hf.manifest, lambda *_: None, emit=printed.append)
    for _ in range(2000):
        r.show("safe\n" * 10)
    assert r.display_dropped > 0
    r.announce(0)
    r.show_overview({"state": "GRACE", "verdict": "PASS", "verified": 1})
    r.emit(r.verdict_line({"state": "GRACE", "verdict": "PASS", "verified": 1}))
    out = "".join(printed)
    assert LOG0 in out and out.endswith("VERDICT PASS state=GRACE verified=1\n"), out[-300:]


def case_stage_lines():
    log = (
        "[box] KF_PAYLOAD_STAGE_START name=cuda_host\n"
        "[box] KF_PAYLOAD_STAGE_END name=cuda_host rc=0 duration_ms=40000\n"
        "KF_PAYLOAD_STAGE_START name=evil;rm -rf\n"
        "KF_PAYLOAD_STAGE_START name=" + "a" * 41 + "\n"
    )
    out, _ = hf.run(log)
    assert "\nSTAGE_START cuda_host (box-reported)\n" in out, out
    assert "\nSTAGE_END cuda_host rc=0 duration_ms=40000 (box-reported)\n" in out
    assert "evil" not in re.sub(
        r"(?m)^\[untrusted\] .*$", "", out
    ), "stage name outside the pattern"
    assert out.count("STAGE_START cuda_host (box-reported)") >= 2, "live and overview"


def case_summary_has_urls():
    printed, summary = [], io.StringIO()
    r = m.Runner(
        hf.ctx,
        hf.manifest,
        lambda *_: (_ for _ in ()).throw(OSError()),
        emit=printed.append,
        summary=summary,
    )
    r.execute()
    assert "| Log URL leg 0 | " + LOG0 + " |" in summary.getvalue(), summary.getvalue()[:600]
    assert "| Status URL leg 0 | " + STATUS0 + " |" in summary.getvalue()


def case_urls_count():
    """Each leg is announced once live and once in the overview: two lines of each, no more."""
    out, _ = hf.run("".join(hf.row(i) for i in range(20)))
    assert len(re.findall(r"^LOG_URL leg=0 ", out, re.M)) == 2, out
    assert len(re.findall(r"^STATUS_URL leg=0 ", out, re.M)) == 2, out


def run_bad(run):
    printed, summary = [], io.StringIO()

    def client(*_):
        raise OSError("stop")

    r = m.Runner(dict(hf.ctx, run=run), hf.manifest, client, emit=printed.append, summary=summary)
    r.execute()
    return "".join(printed), summary.getvalue()


def case_bad_run_execute():
    """A whole execute() with an id that is not validated prints and summarises no URL."""
    for bad in ["123-1/../x", "x\n::stop-commands::", "0-1", "https://evil.example/1-1"]:
        out, summary = run_bad(bad)
        assert (
            "LOG_URL" not in out
            and "STATUS_URL" not in out
            and "evil" not in out.replace("[untrusted]", "")
        ), out
        assert (
            "Log URL" not in summary
            and "Status URL" not in summary
            and "logs.kayfabe" not in summary
        )


def case_bad_leg_announce():
    printed = []
    r = m.Runner(hf.ctx, hf.manifest, lambda *_: None, emit=printed.append)
    for bad in [True, False, -1, 32, 99, "1", None, 2.0]:
        r.announce(bad)
    assert printed == [] and r.announced == {}, printed


def case_verdict_flood_lines():
    """Many short lines exhaust the LINE budget; the verdict and the URLs are still there."""
    out, _ = hf.run("x\n" * 6000)
    assert out.rstrip().splitlines()[-1] == "VERDICT FAIL state=GRACE verified=1"
    assert out.count("LOG_URL leg=0 " + LOG0) >= 2


def case_verdict_small_log():
    out, _ = hf.run("hello\n")
    assert out.rstrip().splitlines()[-1] == "VERDICT FAIL state=GRACE verified=1"


def case_stage_name_unterminated():
    out, _ = hf.run("KF_PAYLOAD_STAGE_START name=evil;rm -rf\n")
    assert "STAGE_START" not in re.sub(r"(?m)^\[untrusted\] .*$", "", out), out


def case_stage_name_long():
    out, _ = hf.run("KF_PAYLOAD_STAGE_START name=" + "a" * 41 + "\n")
    assert "STAGE_START" not in re.sub(r"(?m)^\[untrusted\] .*$", "", out), out


def case_stage_marked():
    out, _ = hf.run(
        "KF_PAYLOAD_STAGE_START name=one\nKF_PAYLOAD_STAGE_END name=one rc=3 duration_ms=5\n"
    )
    lines = re.findall(r"^STAGE_(?:START|END) .*$", out, re.M)
    assert len(lines) >= 4 and all(x.endswith(" (box-reported)") for x in lines), lines


def case_stage_cap():
    out, _ = hf.run("".join("KF_PAYLOAD_STAGE_START name=s%d\n" % i for i in range(300)))
    assert len(re.findall(r"^STAGE_START ", out, re.M)) <= 2 * m.STAGE_LINES, out[-500:]
    live = out.split("OVERVIEW")[0]
    assert len(re.findall(r"^STAGE_START ", live, re.M)) <= m.STAGE_LINES


def case_stage_cap_overview():
    out, _ = hf.run("".join("KF_PAYLOAD_STAGE_START name=s%d\n" % i for i in range(300)))
    assert len(re.findall(r"^STAGE_START ", out.split("OVERVIEW")[1], re.M)) <= m.STAGE_LINES


def case_summary_log_url():
    _, summary = run_bad(RUN)
    assert "| Log URL leg 0 | " + LOG0 + " |" in summary, summary[:500]


def case_summary_status_url():
    _, summary = run_bad(RUN)
    assert "| Status URL leg 0 | " + STATUS0 + " |" in summary, summary[:500]


def masked_main(url):
    """Run main() through round3_main_fixture; apply every ::add-mask:: value like GitHub does."""
    import json, subprocess

    here = Path(__file__).resolve().parent
    p = subprocess.run(
        [
            sys.executable,
            "-I",
            str(here / "round3_main_fixture.py"),
            json.dumps({"url": url, "push": True}),
        ],
        capture_output=True,
        text=True,
        env={
            "PATH": os.environ.get("PATH", ""),
            "KF_RUNNER_PATH": os.environ.get("KF_RUNNER_PATH", ""),
        },
    )
    assert p.returncode == 0, p.stderr
    out = json.loads(p.stdout)["stdout"]
    masks = [x[len("::add-mask::") :] for x in out.splitlines() if x.startswith("::add-mask::")]
    shown = []
    for line in out.splitlines():
        if line.startswith("::add-mask::"):
            continue
        for value in sorted(masks, key=len, reverse=True):
            line = line.replace(value, "***")
        shown.append(line)
    return shown, masks


def check_survives(url):
    shown, masks = masked_main(url)
    text = "\n".join(shown)
    assert "LOG_URL leg=0 https://logs.kayfabe.dev/vast-123-1-0/log.txt" in shown, text[:800]
    assert "STATUS_URL leg=0 https://kayfabe.dev/status/run/?id=vast-123-1-0" in shown, text[:800]
    assert any(x.startswith("VERDICT ") and "***" not in x for x in shown), shown[-3:]
    return masks


def case_mask_survives_ci_origin():
    check_survives("https://ci.kayfabe.dev")


def case_mask_survives_workers_dev_origin():
    check_survives("https://kayfabe-ci.some-account.workers.dev")


def case_mask_no_labels():
    """The origin, hostname and path prefix are masked; no bare hostname label is."""
    masks = check_survives("https://ci.kayfabe.dev")
    assert "https://ci.kayfabe.dev" in masks and "ci.kayfabe.dev" in masks, masks
    for label in ["ci", "kayfabe", "dev"]:
        assert label not in masks, masks
    assert any(len(x) >= 32 and x.startswith("unit-path-only-") for x in masks), masks


def case_mask_secrets_kept():
    import json, subprocess

    here = Path(__file__).resolve().parent
    p = subprocess.run(
        [
            sys.executable,
            "-I",
            str(here / "round3_main_fixture.py"),
            json.dumps({"access": True, "push": True}),
        ],
        capture_output=True,
        text=True,
        env={
            "PATH": os.environ.get("PATH", ""),
            "KF_RUNNER_PATH": os.environ.get("KF_RUNNER_PATH", ""),
        },
    )
    out = json.loads(p.stdout)["stdout"]
    masks = [x[len("::add-mask::") :] for x in out.splitlines() if x.startswith("::add-mask::")]
    for secret in ["runner-a.access", "a" * 64, "mock.jwt.value"]:
        assert secret in masks, secret
    assert any(x.startswith("unit-path-only-") for x in masks)


def case_overview_no_rental_marker():
    """The overview never reuses the live `RENTAL instance=` marker (logged once per leg)."""
    printed = []
    r = m.Runner(hf.ctx, hf.manifest, lambda *_: None, emit=printed.append)
    r.announce(0)
    r.leg_info[0] = "instance=1 host=2 machine=3 state=GRACE verdict=PASS"
    r.show_overview({"state": "GRACE", "verdict": "PASS", "verified": 1})
    assert "RENTAL instance=" not in "".join(printed)
    assert "LEG 0 instance=1" in "".join(printed)


if __name__ == "__main__":
    globals()["case_" + sys.argv[1].replace("-", "_")]()
