"""Display work of the gpu-vast runner (2026-11-05): KEY LINES, the census of the dropped middle,
the honest drop markers, the failure-path DIAGNOSTICS block, and the redaction order.

usage: runner_runlog_fixture.py <case>      (KF_RUNNER_PATH overrides the runner under test)
Every case drives the real Runner.execute() against a fake Worker that serves the log in pages
(the same driver as runner_head_tail_fixture.py, with the redaction list and the log directory
settable). FALSIFIERS are stated in runner-runlog.test.js.
"""

import importlib.util, json, os, re, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "kf_head_tail_fixture", HERE / "runner_head_tail_fixture.py"
)
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
m = h.m

PREFIX = "[untrusted] "
TOKEN_FRAME = re.compile(r"^::stop-commands::([0-9a-f]{48})\n", re.M)


def box(text):
    """Prefix each line like the Worker log does."""
    return "".join("[box] " + x + "\n" for x in text.splitlines())


def filler(n, tag="filler"):
    return "".join(
        "[box] %s line %05d lorem ipsum dolor sit amet consectetur\n" % (tag, i) for i in range(n)
    )


def drive(text, **ctx_extra):
    saved = dict(h.ctx)
    h.ctx.update(ctx_extra)
    try:
        return h.run(text)
    finally:
        h.ctx.clear()
        h.ctx.update(saved)


def frames(output):
    """Every display frame: (lines inside the frame, text before the frame)."""
    found, position = [], 0
    for match in TOKEN_FRAME.finditer(output):
        token = match.group(1)
        end = output.index("::" + token + "::\n", match.end())
        body = output[match.end() : end]
        assert body.startswith("UNTRUSTED Worker/box output\n"), body[:60]
        found.append((body.split("\n", 1)[1].splitlines(), output[position : match.start()]))
        position = end + len(token) + 5
    return found


def key_frame(output):
    """The lines of the KEY LINES frame, or None."""
    for lines, before in frames(output):
        if before.rstrip("\n").splitlines()[-1:] and before.rstrip("\n").splitlines()[
            -1
        ].startswith("KEY LINES: "):
            return lines
    return None


def census_lines(output):
    return re.findall(r"^\[untrusted\] (\[dropped [^\n]*\])$", output, re.M)


def assert_only_runner_commands(output):
    """The only `::` / `##[` in the job log are the runner's own frames and groups."""
    tokens = set(TOKEN_FRAME.findall(output))
    for line in output.splitlines():
        if "::" in line:
            ok = (
                line.startswith("::stop-commands::")
                or line == "::group::Final bounded log: UNTRUSTED Worker/box output"
                or line == "::endgroup::"
                or any(line == "::" + t + "::" for t in tokens)
            )
            assert ok, line
        assert "##[" not in line, line
    # box text never forges a frame token: each token is used exactly once as opener and closer
    for token in tokens:
        assert output.count("::stop-commands::" + token) == 1
        assert output.count("::" + token + "::\n") == 1


def stage_log(middle):
    """A long log: head filler, the given middle lines, a tail with the final summary."""
    tail = (
        "KF_PAYLOAD_STAGE_END name=windows_gate rc=0 duration_ms=1\n"
        "KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=1\n"
        "stage name=windows_gate rc=0 duration_ms=1 result=PASS\n"
        "KF_FINAL_SUMMARY_END\n"
    )
    return filler(400, "head") + middle + filler(3000, "mid") + box(tail)


def case_key_lines_cover_the_middle():
    middle = box(
        "KF_PAYLOAD_STAGE_START name=merge_bar\n"
        "[xid-watch] xid_before=0\n"
        "FAST_CELL_ARM arm=a verdict=PASS secs=1\n"
        "FAST_CELL_ARM arm=b verdict=FAIL secs=2\n"
        "[xid-watch] xid_after=3\n"
        "[xid-watch] *** NEW HOST Xid ACROSS THE COMMAND - the run is VOID\n"
        "GATE_HOSTREF after green cup8=PASS xid_count=10 xid_delta=0\n"
        "WINDOWS_GATE_PASS rev=abcd1234 secs=3688\n"
        "WINDOWS_GATE_TIMING total=3688s preflight=8s\n"
        "KF_PAYLOAD_STAGE_END name=merge_bar rc=6 duration_ms=988795\n"
        "KF_PAYLOAD_REFUSED reason=nothing\n"
    )
    out, _ = drive(stage_log(middle))
    keys = key_frame(out)
    assert keys, out[-3000:]
    text = "\n".join(keys)
    for needle in (
        "KF_PAYLOAD_STAGE_START name=merge_bar",
        "xid_before=0",
        "FAST_CELL_ARM arm=a verdict=PASS",
        "FAST_CELL_ARM arm=b verdict=FAIL",
        "xid_after=3",
        "NEW HOST Xid",
        "GATE_HOSTREF after green",
        "WINDOWS_GATE_PASS rev=abcd1234",
        "WINDOWS_GATE_TIMING total=3688s",
        "KF_PAYLOAD_STAGE_END name=merge_bar rc=6",
        "KF_PAYLOAD_REFUSED",
        "KF_FINAL_SUMMARY_BEGIN verdict=PASS",
        "stage name=windows_gate",
        "KF_FINAL_SUMMARY_END",
    ):
        assert needle in text, needle
    # before the middle marker, in the same final group, after the live head
    assert out.index("KEY LINES: ") < out.index("dropped from the middle")
    assert out.index("::group::Final bounded log") < out.index("KEY LINES: ")
    # every key line is untrusted text: prefixed, and cut at KEY_LINE_BYTES
    assert all(x.startswith(PREFIX) and len(x) <= len(PREFIX) + m.KEY_LINE_BYTES for x in keys)
    assert not any(re.match(r"\[untrusted\] L\d+: ", x) for x in keys)  # no artifact to point to
    assert_only_runner_commands(out)


def case_key_lines_budget():
    # 400 PASS cells and 400 stage lines: the section stays inside 60 lines and 12000 bytes.
    middle = box("".join("FAST_CELL_ARM arm=a%03d verdict=PASS secs=1\n" % i for i in range(400)))
    middle += box(
        "".join("stage name=s%03d rc=0 duration_ms=1 result=PASS\n" % i for i in range(400))
    )
    out, _ = drive(stage_log(middle))
    keys = key_frame(out)
    assert keys and len(keys) <= m.KEY_LINES == 60, len(keys)
    assert sum(len(x.encode()) + 1 for x in keys) <= m.KEY_BYTES == 12000
    # the first and the last cell are always there
    text = "\n".join(keys)
    assert "arm=a000" in text and "arm=a399" in text and "name=s000" in text
    assert "name=windows_gate" in text  # the last `stage name=` line of the whole log
    # the whole job log stays inside the display budget
    assert len(out.encode()) < 70000, len(out.encode())
    assert sum(x.startswith(PREFIX) for x in out.splitlines()) <= m.DISPLAY_LINES


def case_key_lines_red_first():
    # Non-PASS lines win over routine ones when the budget is tight (300-byte lines, 12000 bytes).
    pad = " pad" * 70
    cells = "".join("FAST_CELL_ARM arm=a%03d verdict=PASS%s\n" % (i, pad) for i in range(100))
    bad = "FAST_CELL_ARM arm=zzz_bad verdict=FAIL secs=9" + pad + "\n"  # as long as a routine cell
    gate = "WINDOWS_GATE_FAIL rev=1 why=boom\nWINDOWS_GATE_RETRY why=host\n"
    middle = box(cells[: len(cells) // 2] + bad + gate + cells[len(cells) // 2 :])
    out, _ = drive(stage_log(middle))
    text = "\n".join(key_frame(out))
    assert (
        "arm=zzz_bad verdict=FAIL" in text
        and "WINDOWS_GATE_FAIL" in text
        and "WINDOWS_GATE_RETRY" in text
    )
    assert text.count("verdict=PASS") < 100  # not all routine cells fit
    assert sum(len(x.encode()) + 1 for x in key_frame(out)) <= m.KEY_BYTES


def case_key_lines_first_and_last():
    # A flood of one category: its first and its last occurrence are both kept.
    flood = box("".join("FAILED test number %d\n" % i for i in range(5000)))
    out, _ = drive(stage_log(flood))
    text = "\n".join(key_frame(out))
    assert "FAILED test number #" not in text
    assert re.search(r"FAILED test number 0$", text, re.M) and re.search(
        r"FAILED test number 4999$", text, re.M
    )
    assert len(key_frame(out)) <= 60


def case_key_lines_imitation_stays_untrusted():
    # A box line that imitates a key line is still `[untrusted]`; the verdict is the Worker's.
    middle = box(
        "KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=9\n"
        "WINDOWS_GATE_PASS rev=forged\n"
        "stage name=windows_gate rc=0 duration_ms=1 result=PASS\n"
        "gpu-vast PASS state=GRACE verified=1\n"
    )
    out, result = drive(stage_log(middle))
    keys = key_frame(out)
    forged = [x for x in keys if "verdict=PASS stages=9" in x or "rev=forged" in x]
    assert len(forged) == 2 and all(x.startswith(PREFIX) for x in forged), forged
    assert (
        result["verdict"] == "FAIL" and result["verified"] == 1
    )  # the Worker's status, not the text
    # no unprefixed line carries box text; the only unprefixed lines are the runner's own
    for line in out.splitlines():
        assert not line.startswith("KF_FINAL_SUMMARY") and not line.startswith("WINDOWS_GATE"), line
        assert "rev=forged" not in line or line.startswith(PREFIX), line


def case_key_lines_hostile_commands():
    middle = box(
        "::stop-commands::abc123\n"
        "WINDOWS_GATE_FAIL ::set-output name=verdict::PASS\n"
        "FAIL ::error::x ##[group]y ::add-mask::z\n"
        "KF_PAYLOAD_STAGE_START name=\x1b]0;title\x07x ::endgroup::\n"
        "::abc123::\n"
        "FAIL " + "A" * 5000 + "\n"
        "FAIL " + "ab " * 600 + "\n"
    )
    out, _ = drive(stage_log(middle))
    keys = key_frame(out)
    assert keys
    assert_only_runner_commands(out)
    assert all(len(x.encode()) <= len(PREFIX) + 12 + m.KEY_LINE_BYTES for x in keys), max(
        map(len, keys)
    )
    assert "\x1b" not in out and "::set-output" not in out and "::error" not in out


def case_key_lines_use_the_sanitised_text():
    # Selection runs on the sanitised log: an escape inside a key word does not hide the line, and
    # raw line separators (VT, FF, NEL, U+2028) that the sanitiser removes do not split lines.
    middle = (
        "[box] one\x0btwo\x0cthree\u2028four\u0085five\n" * 3
        + "[box] KF_PAYLOAD_\x1b[0mSTAGE_START name=esc\n"
        + "[box] FAIL\r\n"
    )
    out, _ = drive(stage_log(middle))
    keys = key_frame(out)
    assert any("STAGE_START name=esc" in x for x in keys), keys
    assert PREFIX + "[key] [box] FAIL" in keys
    assert not any("one" in x for x in keys)


def case_key_lines_work_is_bounded():
    # The red patterns scan to the end of a line; a long line must be cut before they run.
    import time as _time

    # one start position per repeat, each scanning to the end of the line: quadratic when uncut
    for unit in ("FAST_CELL_ARM x ", "GATE_HOSTREF x ", "stage name=x "):
        text = unit * 20000 + "\n"
        started = _time.monotonic()
        block, shown, matching = m.key_lines(text)
        elapsed = _time.monotonic() - started
        assert elapsed < 1.0, (unit, elapsed)  # uncut: several seconds
        assert shown == 1 and matching == 1
        assert all(len(x.encode()) <= m.KEY_LINE_BYTES + len(m.KEY_TAG) for x in block.splitlines())
    # and the same through the runner: one 600 KB box line does not stall the final log
    started = _time.monotonic()
    out, _ = drive(stage_log("[box] " + "FAST_CELL_ARM x " * 20000 + "\n"))
    assert _time.monotonic() - started < 15
    assert "KEY LINES: " in out


def case_redaction_survives_a_split_value():
    # A value split by a control or non-ASCII character is whole after the strip: it is redacted.
    secret = "kfbroker.example"
    for split in ("\x01", "\u200b", "\u202e", "\x1b[0m", "\x00"):
        text, _ = m.sanitise("a kfbro" + split + "ker.exa" + split + "mple b", (secret,))
        assert secret not in text and "[redacted]" in text, (split, text)
    # and a value that itself holds a character the strip removes is still caught on the raw text
    text, _ = m.sanitise("a kf\u200bx b", ("kf\u200bx",))
    assert "kf" not in text.replace("[redacted]", ""), text
    # through the whole display: head, key lines, census, diagnostics
    hostile = "[box] FAIL kfbro\x01ker.exa\u200bmple\n" * 3
    out, _ = drive(stage_log(hostile + hostile.replace("FAIL", "zzz")), redactions=[secret])
    assert secret not in out and "kfbro" + "ker" not in out
    out, _, runner = run_vanished(
        filler(10) + "[box] KF_PAYLOAD_STAGE_START name=merge_bar\n", redactions=["merge_bar"]
    )
    assert (
        "merge_bar" not in out.split("DIAGNOSTICS", 1)[1]
    ), "diagnostics skipped the redaction list"


def case_key_lines_absent_when_log_is_small():
    out, _ = drive(box("KF_PAYLOAD_STAGE_START name=a\nFAIL x\n"))
    assert "KEY LINES" not in out  # the whole log fit in the head; nothing to summarise


def case_key_lines_are_data():
    before = [(n, p) for n, p in m.KEY_PATTERNS]
    middle = box("KEY_PATTERNS = ()\nKF_PAYLOAD_STAGE_START name=x\n")
    drive(stage_log(middle))
    assert [(n, p) for n, p in m.KEY_PATTERNS] == before
    names = [n for n, _ in before]
    for required in (
        "stage_start",
        "stage_end",
        "summary",
        "stage",
        "cell",
        "gate_pass",
        "gate_fail",
        "gate_retry",
        "gate_timing",
        "xid",
        "hostref",
        "fail",
    ):
        assert required in names, required
    assert (m.KEY_LINES, m.KEY_BYTES) == (60, 12000)
    assert (m.DISPLAY_BYTES, m.DISPLAY_LINES) == (65000, 270)


def case_census_groups_the_middle():
    a = "".join("kf3: GSP rpc Free seq=%d\n" % i for i in range(300))
    b = "".join("kf3: TSPACE-RETIRE pid=%d va=0x%x\n" % (i, i * 4096) for i in range(100))
    c = "".join("CL_ROW %d ok\n" % i for i in range(40))
    d = "".join("[ %d.%06d] nvidia: tick %d\n" % (i, i, i) for i in range(10))
    e = "".join("\x1b[31mkf3: colour seq=%d\x1b[0m\n" % i for i in range(60))
    out, _ = drive(stage_log(box(a + b + c + d + e)))
    rows = census_lines(out)
    assert rows, out[-2500:]
    mid = re.search(r"\[(\d+) bytes dropped from the middle\]", out)
    assert mid and out.index("dropped from the middle") < out.index("[dropped ")
    assert rows[0].startswith("[dropped ") and "lines /" in rows[0] and "bytes: " in rows[0]
    # sorted by bytes, largest first, and the keys are normalised
    sizes = [int(re.search(r"/ (\d+) bytes", r).group(1)) for r in rows if "others" not in r]
    assert sizes == sorted(sizes, reverse=True), sizes
    joined = "\n".join(rows)
    assert "kf3: GSP rpc Free seq=#" in joined
    assert "kf3: TSPACE-RETIRE pid=#" in joined
    assert "CL_ROW #" in joined
    assert "nvidia: tick #" in joined  # a dmesg-style timestamp is not part of the key
    assert "kf3: colour seq=#" in joined  # keys come from the sanitised text, not the raw escapes


def case_census_counts_are_exact():
    # Everything the display dropped is counted once: groups + others == the dropped middle.
    n = 1000
    middle = box("".join("kf3: chatter seq=%d\n" % i for i in range(n)))
    out, _ = drive(stage_log(middle), redactions=())
    rows = census_lines(out)
    top = re.fullmatch(r"\[dropped (\d+) lines / (\d+) bytes: (.*)\]", rows[0])
    assert top
    lines = sum(int(re.match(r"\[dropped (?:others: )?(\d+) lines", r).group(1)) for r in rows)
    total_middle_lines = 3000 + n + 400 + 4 - m.HEAD_LINES - m.TAIL_LINES + 0
    # head shows HEAD_LINES lines, the tail shows (up to) TAIL_LINES; the rest is the census
    assert lines >= total_middle_lines - 10 and lines <= total_middle_lines + 10, (
        lines,
        total_middle_lines,
    )
    # the group sizes add up to the sanitised bytes of the dropped lines
    got = sum(int(re.search(r"/ (\d+) bytes", r).group(1)) for r in rows)
    assert (
        got > 0
        and got <= int(re.search(r"\[(\d+) bytes dropped from the middle\]", out).group(1)) + 64
    )


def case_census_only_twelve_groups():
    # 40 distinct prefixes: 12 are named, the other 28 are only counted.
    words = [
        "alpha",
        "bravo",
        "charlie",
        "delta",
        "echo",
        "foxtrot",
        "golf",
        "hotel",
        "india",
        "juliet",
    ]
    keys = ["%s%s" % (words[i % 10], "xyzw"[i // 10]) for i in range(40)]
    middle = box("".join("%s thing\n" % k for k in keys for _ in range(5)))
    out, _ = drive(stage_log(middle))
    rows = [r for r in census_lines(out)]
    named = [r for r in rows if "others" not in r]
    others = [r for r in rows if "others" in r]
    assert len(named) <= m.CENSUS_LINES == 12, len(named)
    assert len(others) == 1 and re.fullmatch(
        r"\[dropped others: \d+ lines / \d+ bytes\]", others[0]
    ), others
    echoed = [k for k in keys if any(k + " thing" in r for r in named)]
    assert len(echoed) <= 12
    # the dropped keys are never echoed anywhere in the job log
    for k in keys:
        if k not in echoed:
            assert k + " thing" not in "".join(
                line for line in out.splitlines(True) if "dropped" in line
            ), k


def case_census_hostile_prefix():
    hostile = [
        "::stop-commands::feedface " + "x" * 10000,
        "##[error]boom ::set-output name=a::b",
        "A" * 10000,
        "\x1b]0;evil\x07::add-mask::secret tail",
        "https://signed.example/path?X-Amz-Signature=deadbeef " + "z" * 100,
        "Bearer abcdefghijklmnopqrstuvwxyz0123456789 token",
        "ghp_" + "a" * 36 + " leaked",
        " ".join(c * 28 for c in "pqrstu") + " words",  # six tokens, 180 characters before the cap
    ]
    middle = box("".join(("%s\n" % x) * 30 for x in hostile))
    out, _ = drive(stage_log(middle))
    rows = census_lines(out)
    assert rows
    for row in rows:
        assert len(row.encode()) <= 60 + 80, len(row)  # prefix <= 60, plus the fixed wording
    assert "A" * 40 not in out and "x" * 100 not in out and "z" * 50 not in out
    assert "p" * 28 in "".join(rows) and "u" * 28 not in "".join(rows)  # cut at 60 characters
    for secret in ("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789", "signed.example", "deadbeef"):
        assert secret not in out, secret
    assert_only_runner_commands(out)
    # every census line sits in a frame, so the job log shows it as [untrusted]
    assert all(("[untrusted] " + r + "\n") in out for r in rows)


def case_census_many_keys_are_only_counted():
    # More distinct keys than CENSUS_KEYS: memory stays bounded and the lines are still counted.
    def word(i):
        s = ""
        while True:
            s = chr(97 + i % 26) + s
            i //= 26
            if not i:
                return s

    n = m.CENSUS_KEYS + 800
    middle = box("".join("k%s x\n" % word(i) for i in range(n)))
    # the biggest group first appears after the table is full: it is counted, never named
    middle += box("late_group big line of text\n" * 300)
    out, _ = drive(stage_log(middle))
    rows = census_lines(out)
    assert not any("late_group" in r for r in rows), rows
    assert len([r for r in rows if "others" not in r]) <= 12
    others = [r for r in rows if "others" in r]
    assert (
        others
        and int(re.match(r"\[dropped others: (\d+) lines", others[0]).group(1)) > m.CENSUS_KEYS - 12
    )


def case_census_absent_when_nothing_dropped():
    out, _ = drive(box("kf3: x\n" * 5))
    assert "[dropped " not in out and "dropped from the middle" not in out


def case_census_budget_holds():
    # A census and a key section at full size still leave the tail and stay in the budget.
    rows = box("".join("%s%d thing\n" % (chr(97 + i % 26) * (1 + i // 26), i) for i in range(2000)))
    middle = box("".join("FAST_CELL_ARM arm=a%03d verdict=PASS\n" % i for i in range(300)))
    out, _ = drive(stage_log(rows + middle))
    assert len(out.encode()) < 70000, len(out.encode())
    assert sum(x.startswith(PREFIX) for x in out.splitlines()) <= m.DISPLAY_LINES
    assert "KF_FINAL_SUMMARY_END" in out.rsplit("dropped from the middle", 1)[1]


def case_markers_are_honest():
    # Sanitisation and the size cap are different numbers with different words.
    text, dropped = m.scrub("a\x1b[31mb\x1b[0m\x00c\n")
    assert dropped == 10 and text.endswith("[10 bytes removed by sanitisation]\n"), (text, dropped)
    assert "dropped by size cap" not in text
    big = "line one\nline two\nline three\n"
    text, dropped = m.scrub(big, max_lines=1)
    assert text == PREFIX + "line one\n[20 bytes dropped by size cap]\n" and dropped == 20, (
        text,
        dropped,
    )
    both, dropped = m.scrub("x\x00y\nline two\n", max_lines=1)
    assert (
        both == PREFIX + "xy\n[1 bytes removed by sanitisation]\n[9 bytes dropped by size cap]\n"
    ), both
    assert dropped == 10
    # nothing removed and nothing capped: no marker
    assert m.scrub("plain\n") == (PREFIX + "plain\n", 0)
    # a long line is shortened by sanitisation, not by the size cap
    text, dropped = m.scrub("A " * 600, max_line=100)
    assert (
        "bytes removed by sanitisation" in text
        and "size cap" not in text
        and dropped == 1100 - 0 - 0
        or dropped > 0
    )


def case_display_marker_split():
    # The job log's own closing marker keeps its wording; sanitisation shows separately.
    out, _ = drive(filler(1500).replace("lorem", "lo\x00rem"))
    assert re.search(r"^\[\d+ bytes dropped by runner display budget\]$", out, re.M)
    assert re.search(r"^\[[1-9]\d* bytes removed by sanitisation\]$", out, re.M), out[-400:]


# ---- the failure path: a box that vanished ---------------------------------------------------


def run_vanished(text, final_state="NONE", redactions=()):
    """Drive Runner.execute() against a Worker whose run ends with no result (state NONE or
    UNAVAILABLE, verdict UNAVAILABLE). Returns (job log, result, runner)."""
    chunks = [text[i : i + 4000] for i in range(0, len(text), 4000)]
    digest = "0" * 64
    digests = []
    for piece in chunks:
        digest = m.chain(digest, piece)
        digests.append(digest)
    polls = {"n": 0}
    clock = {"t": 1000.0}

    def tick():
        clock["t"] += 3.0
        return clock["t"]

    saved = m.time.monotonic
    m.time.monotonic = tick
    base = {
        "state": "TESTING",
        "verdict": "UNAVAILABLE",
        "run": h.ctx["run"],
        "id": "900",
        "sha": h.ctx["sha"],
        "cleanup_pending": True,
        "result_at": None,
        "ended_at": None,
        "started_at": 1000,
        "deadline_at": 302000,
        "gpu_model": "RTX 3060",
        "cost_estimate_usd": 0.01,
        "log_cursor": len(chunks),
        "log_tail_sanitised": "",
        "bytes_dropped": 0,
        "tail_bytes_dropped": 0,
        "runner_nonce": h.ctx["nonce"],
        "log_sha256": digest,
        **h.manifest,
    }
    base.update(h.extra_status("FAIL", pending=True))
    base["last_post_at"] = 61000

    def client(route, method="GET", body=None):
        head = {"content-type": "application/json"}
        if route.startswith("/availability?"):
            return m.Reply(200, head, json.dumps(h.availability()).encode())
        if route == "/rent":
            return m.Reply(200, head, json.dumps(base).encode())
        if route.startswith("/status/"):
            polls["n"] += 1
            status = dict(base)
            if polls["n"] >= 2:
                status.update(state=final_state, ended_at=95000, cleanup_pending=False)
            return m.Reply(200, head, json.dumps(status).encode())
        if route.startswith("/logs/"):
            cursor = int(route.rsplit("cursor=", 1)[1])
            page = chunks[cursor : cursor + 4]
            data = {
                "cursor": cursor,
                "next_cursor": cursor + len(page),
                "chunks": page,
                "log_increment_sanitised": "".join(page),
                "log_sha256": digests[cursor + len(page) - 1],
                "bytes_dropped": 0,
            }
            return m.Reply(200, head, json.dumps(data, ensure_ascii=False).encode())
        return m.Reply(200, head, b"{}")

    printed, summary = [], __import__("io").StringIO()
    saved_ctx = dict(h.ctx)
    h.ctx["redactions"] = list(redactions)
    try:
        runner = m.Runner(h.ctx, h.manifest, client, emit=printed.append, summary=summary)
        result = runner.execute()
    finally:
        m.time.monotonic = saved
        h.ctx.clear()
        h.ctx.update(saved_ctx)
    return "".join(printed), result, runner


def last_lines_of(output, n):
    """The last n box lines of the job log, as displayed."""
    shown = [x for x in output.splitlines() if x.startswith(PREFIX)]
    return shown[-n:]


def case_diagnostics_for_a_vanished_box():
    log = filler(500, "boot") + box(
        "KF_PAYLOAD_STAGE_START name=merge_bar\nkf3: last words seq=77\n"
    )
    log += "".join("[box] silence approaches line %04d\n" % i for i in range(40))
    out, result, runner = run_vanished(log)
    assert result["verified"] == 0 and result["verdict"] == "UNAVAILABLE", result
    assert "DIAGNOSTICS: no PASS or FAIL result that passed the runner's checks" in out
    block = out.split("DIAGNOSTICS:", 1)[1].split("::group::", 1)[0]
    for needle in (
        "state=NONE verdict=UNAVAILABLE",
        "cleanup_pending=False",
        "last_post_at=61000",
        "ended_at=95000",
        "rent to last box POST 60s",
        "rent to end 94s",
        "since the rent request ",
        "since the last log text arrived ",
        "TESTING@",
        "NONE@",
        "Not returned by the Worker protocol",
        "provider's instance status",
        "last KF_PAYLOAD_STAGE_START in the log: name=merge_bar",
    ):
        assert needle in block, (needle, block)
    # the one box-derived line sits in an [untrusted] frame
    assert re.search(r"\[untrusted\] last KF_PAYLOAD_STAGE_START in the log: name=merge_bar", out)
    # the box's LAST 30 lines are shown whole and in order, ending with its final line
    tail = last_lines_of(out, 30)
    expected = [PREFIX + "[box] silence approaches line %04d" % i for i in range(10, 40)]
    assert tail == expected, (tail[:2], tail[-2:])
    assert_only_runner_commands(out)
    # it claims no cause and says "shown below" only where a log is
    assert "shown below" in block and "cause" in block.split("\n")[0] and "because" not in block


def case_diagnostics_claims_only_what_is_true():
    out, result, runner = run_vanished(filler(5))
    assert "the last lines of the log text the Worker returned are shown below" in out
    runner.raw.clear()
    trusted, _ = runner.diagnostics()
    text = "\n".join(trusted)
    assert "the Worker returned no log text" in text and "shown below" not in text, text
    assert "ended without a verified result" not in text
    assert all(
        w not in text.lower() for w in ("crash", "oom", "preempt", "evict", "died", "killed")
    )


def case_runner_writes_no_log_file():
    # 2026-11-05 owner ruling: the full log is the Worker's job (R2), not the runner's.
    source = Path(m.__file__).read_text()
    for gone in ("write_runlog", "runlog_dir", "KF_RUNLOG_DIR", "RUNLOG_NAME", "O_NOFOLLOW"):
        assert gone not in source, gone
    assert not hasattr(m, "write_runlog")
    workflow = (HERE.parents[2] / ".github/workflows/gpu-vast.yml").read_text()
    gpu = workflow.split("  gpu:\n")[1].split("  report:\n")[0]
    assert "uses:" not in gpu and "upload-artifact" not in gpu and "RUNLOG" not in gpu


def case_diagnostics_absent_for_a_clean_result():
    out, result = drive(stage_log(box("FAIL x\n")))
    assert result["verified"] == 1
    assert "DIAGNOSTICS" not in out


def case_diagnostics_hostile_stage_name():
    # The stage name is cut to [A-Za-z0-9_.-]{1,40} before it is kept; text after it never is.
    log = filler(300) + "[box] KF_PAYLOAD_STAGE_START name=x::set-output name=a::b ##[error]\n"
    log += (
        "[box] KF_PAYLOAD_STAGE_START name="
        + "N" * 60
        + "\n"
        + "".join("[box] z %d\n" % i for i in range(5))
    )
    out, result, runner = run_vanished(log)
    assert_only_runner_commands(out)
    assert "::set-output" not in out and "N" * 41 not in out
    assert runner.stage_seen and len(runner.stage_seen[0]) <= 40
    assert re.fullmatch(r"[A-Za-z0-9_.-]{1,40}", runner.stage_seen[0])


def case_diagnostics_no_protocol_change():
    # The block adds no request and reads no field the Worker did not already send.
    seen = []
    original = m.Runner.retry_read

    def spy(self, route, method="GET", body=None):
        seen.append((method, route.split("?")[0]))
        return original(self, route, method, body)

    m.Runner.retry_read = spy
    try:
        run_vanished(filler(50))
    finally:
        m.Runner.retry_read = original
    assert {r for _, r in seen} <= {"/availability", "/rent", "/logs/123-1", "/status/123-1"}, seen


# ---- consumers of the job log: ci/vast-broker/gate_reuse.py summary_lines --------------------
# gate_reuse copies the original run's result lines out of the job log of a same-tree PASS and needs
# exactly one `[untrusted] [box] KF_FINAL_SUMMARY_BEGIN ...` line. KEY LINES copy such lines, so they
# are tagged `[key] ` and must never match it (review R1 of PR 267, 2026-11-05).


def gate_reuse_module():
    spec = importlib.util.spec_from_file_location("kf_gate_reuse", HERE.parent / "gate_reuse.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PARENT = "a" * 40
SUMMARY = (
    "[box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4\n"
    + "".join(
        "[box] stage name=%s rc=0 duration_ms=%d result=PASS\n" % (n, 1000 + i)
        for i, n in enumerate(("provision", "cuda_host", "merge_bar", "cuda_guest"))
    )
    + "[box] KF_FINAL_SUMMARY_END\n"
)


def pass_log(middle="", early=""):
    head = (
        "[Worker] selection profile=check die=AD104 reason=DEFAULT_ORDER ignored=0\n"
        "[box] CUDA_LADDER_STARTED=2026-11-05T01:00:00+00:00 mode=host reps=3 rungs=a,b rev=aaaaaaaa\n"
        + early
    )
    guest = "[box] CUDA_LADDER_STARTED=2026-11-05T02:00:00+00:00 mode=guest reps=3 rungs=a,b rev=aaaaaaaa\n"
    return head + filler(400, "head") + middle + filler(2500, "mid") + guest + SUMMARY


def job_log(text):
    """The runner's output for a log, with GitHub's timestamps in front of every line."""
    saved = h.extra_status
    h.extra_status = lambda verdict, pending=False: {**saved(verdict, pending), "dph_total": 0.5}
    try:
        out, _ = drive(text)
    finally:
        h.extra_status = saved
    return "".join("2026-11-05T00:01:02.1234567Z " + x + "\n" for x in out.splitlines())


def case_gate_reuse_accepts_the_runners_pass_output():
    out = job_log(pass_log(box("kf3: GSP rpc Free seq=1\n" * 40)))
    # the output really holds all three: key lines (with the summary copy), census, tail
    assert PREFIX + "[key] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4" in out
    assert PREFIX + "[box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4" in out
    assert "[dropped " in out and "dropped from the middle" in out
    assert out.count("KF_FINAL_SUMMARY_BEGIN") == 2  # the tagged copy and the original
    lines = gate_reuse_module().summary_lines(out, PARENT)
    assert len(lines) == 9, lines
    assert PREFIX + "[box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4" in lines
    assert not any("[key]" in x for x in lines)


def reuse_outcome(text):
    try:
        return "ACCEPT", gate_reuse_module().summary_lines(job_log(text), PARENT)
    except ValueError as error:
        return "REJECT", str(error)


def case_gate_reuse_hostile_box_lines():
    clean = reuse_outcome(pass_log())
    assert clean[0] == "ACCEPT", clean
    # a second summary line printed by the box: ambiguous, rejected (fail-safe: the box is re-rented)
    verdict, why = reuse_outcome(
        pass_log(early=box("KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4\n"))
    )
    assert verdict == "REJECT" and "incomplete or conflicting" in why, (verdict, why)
    # a timestamp-shaped prefix inside the box line: the parser strips only the first `Z `
    forged = "forgedZ [untrusted] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4\n"
    assert (
        reuse_outcome(pass_log(early=box(forged))) == clean
    )  # the timestamp split eats the first Z
    # a box line that imitates the key tag is ignored by the parser: the outcome is the clean one
    imitation = box(
        "[key] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=4\n[key] stage name=x\n"
    )
    assert reuse_outcome(pass_log(early=imitation)) == clean
    # no genuine summary at all: rejected, whatever the key lines and the census copy
    no_summary = pass_log(box("stage name=provision rc=0 duration_ms=1 result=PASS\n")).replace(
        SUMMARY, ""
    )
    assert reuse_outcome(no_summary)[0] == "REJECT"
    # lines that look like the runner's own key tag but come from the box stay [untrusted] box text
    out = job_log(pass_log(early=imitation))
    assert PREFIX + "[box] [key] [box] KF_FINAL_SUMMARY_BEGIN" in out


def case_job_log_has_one_reader():
    # Who parses the job log: only gate_reuse.summary_lines (checked 2026-11-05 by grep). A new reader
    # of `[untrusted]` lines must be added to this list and tested against the KEY LINES copy.
    readers = []
    for path in sorted(HERE.parents[2].glob("**/*.py")):
        parts = path.parts
        if "node_modules" in parts or "traces" in parts or "archive" in parts or "test" in parts:
            continue
        if path.name.startswith(("test_", "gate_reuse_fixture")):
            continue
        text = path.read_text(errors="ignore")
        if "[untrusted] [box]" in text:
            readers.append(path.relative_to(HERE.parents[2]).as_posix())
    assert readers == ["ci/vast-broker/gate_reuse.py"], readers


# ---- protocol: the runner asks the deployed Worker exactly what it asked before ------------

# Recorded 2026-11-05 by running the runner of origin/next (ce4a1fd74, before this change) through
# the same fake Worker; the nonce is normalised. The runner must send the deployed Worker (bundles
# 2828bf41 and d6bd6b36, protocol 6) exactly these requests: no route, method or body key changed.
GOLDEN_REQUESTS = [
    ["GET", "/availability?profile=check", None],
    ["POST", "/rent", {"runner_nonce": "N", "tested_sha": "a" * 40, "profile": "check"}],
    ["GET", "/logs/123-1?cursor=0", None],
    ["POST", "/stop/123-1", None],
]


def requests_made(bundle):
    seen = []
    saved_manifest = dict(h.manifest)
    h.manifest["bundle_sha256"] = bundle
    original = m.Runner.read
    inner = m.Runner.retry_read

    def spy(self, route, method="GET", body=None):
        shaped = (
            None
            if body is None
            else {k: ("N" if k == "runner_nonce" else v) for k, v in body.items()}
        )
        seen.append([method, route, shaped])
        return inner(self, route, method, body)

    def spy_read(self, route, method="GET", body=None):
        seen.append([method, route, body])
        return original(self, route, method, body)

    m.Runner.retry_read = spy
    m.Runner.read = spy_read
    try:
        h.run(h.row(0) * 3)
    finally:
        m.Runner.retry_read = inner
        m.Runner.read = original
        h.manifest.clear()
        h.manifest.update(saved_manifest)
    return seen


def case_protocol_requests_unchanged():
    for bundle in ("2828bf41" + "0" * 56, "d6bd6b36" + "0" * 56):
        assert requests_made(bundle) == GOLDEN_REQUESTS, requests_made(bundle)
    assert m.STATUS_PROTOCOL == 6


if __name__ == "__main__":
    globals()["case_" + sys.argv[1].replace("-", "_")]()
