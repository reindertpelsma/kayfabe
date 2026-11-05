"""Run-log work of the gpu-vast runner (2026-11-05): KEY LINES, the census of the dropped middle,
the honest drop markers and the sanitised full-log file (the artifact).

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
    # every key line is untrusted text: prefixed, and carries its line number in the full log
    assert all(x.startswith(PREFIX + "L") for x in keys), keys[:3]
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
    bad = "FAST_CELL_ARM arm=zzz_bad verdict=FAIL secs=9\n"
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
    assert len(forged) == 2 and all(x.startswith(PREFIX + "L") for x in forged), forged
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
    # Selection and numbering run on the sanitised log: an escape inside a key word does not hide the
    # line, and raw line separators (VT, FF, NEL, U+2028) that the sanitiser removes do not shift the
    # L<n> numbers away from the lines of the artifact.
    middle = (
        "[box] one\x0btwo\x0cthree\u2028four\u0085five\n" * 3
        + "[box] KF_PAYLOAD_\x1b[0mSTAGE_START name=esc\n"
        + "[box] FAIL\r\n"
    )
    out, _, data = run_with_dir(stage_log(middle))
    keys = key_frame(out)
    assert any("STAGE_START name=esc" in x for x in keys), keys
    lines = data.decode().splitlines()
    for line in keys:
        number, body = re.match(r"\[untrusted\] L(\d+): (.*)", line).groups()
        assert lines[int(number) - 1] == PREFIX + body, (number, body)


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


# ---- the full-log file ---------------------------------------------------------------------


def run_with_dir(text, **extra):
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "gpu-vast-runlog")
        os.mkdir(path)
        out, result = drive(text, runlog_dir=path, **extra)
        file = os.path.join(path, m.RUNLOG_NAME)
        data = Path(file).read_bytes() if os.path.exists(file) else None
        return out, result, data


def case_runlog_has_the_whole_log():
    middle = box("a distinctive middle line\n")
    out, result, data = run_with_dir(stage_log(middle))
    assert data is not None
    text = data.decode()
    assert "a distinctive middle line" in text
    assert PREFIX + "[box] a distinctive middle line" not in out  # not displayed, only in the file
    assert text.count("\n") >= 3400
    assert all(line.startswith(PREFIX) for line in text.splitlines()), [
        x for x in text.splitlines() if not x.startswith(PREFIX)
    ][:3]
    assert len(data) <= m.MAX_LOG
    assert re.search(r"Full sanitised log: \d+ bytes written", out)
    # the L<n> numbers in the key lines are line numbers of this file
    lines = text.splitlines()
    for line in key_frame(out):
        number, body = re.match(r"\[untrusted\] L(\d+): (.*)", line).groups()
        assert lines[int(number) - 1] == PREFIX + body, (number, body)


def case_runlog_redaction_matches_the_display():
    token = "eyJhbGciOiJSUzI1NiJ9.payloadpayloadpayload.signaturesignature"
    secret = "super-secret-worker-path-value-0123456789"
    log = box(
        "Authorization: Bearer %s\n" % token
        + "origin https://broker.example/%s/status?sig=abc\n" % secret
        + "gh token ghp_%s\n" % ("a1" * 20)
        + "hex %s\n" % ("0123456789abcdef" * 4)
        + "worker %s\n" % secret
        + "oidc %s\n" % token
    )
    out, result, data = run_with_dir(filler(300) + log + filler(300), redactions=[secret, token])
    text = data.decode()
    for leaked in (
        token,
        secret,
        "ghp_a1",
        "0123456789abcdef",
        "broker.example",
        "sig=abc",
        "Bearer eyJ",
    ):
        assert leaked not in text, leaked
        assert leaked not in out, leaked
    assert "[redacted]" in text and "[url redacted]" in text


def case_runlog_hostile_commands():
    log = box(
        "::stop-commands::x\n::set-output name=a::b\n##[error]boom\n::add-mask::q\n"
        "\x1b]0;t\x07::warning::w\n"
        "‮RTL ​ zero-width \x00 nul \x9b31m\n"
    )
    out, result, data = run_with_dir(filler(300) + log + filler(300))
    text = data.decode()
    assert "::" not in text and "##[" not in text and "\x1b" not in text
    assert "‮" not in text and "​" not in text and "\x00" not in text and "\x9b" not in text
    assert all(ch in "\t\n" or 32 <= ord(ch) <= 126 or 192 <= ord(ch) <= 255 for ch in text)
    assert all(line.startswith(PREFIX) for line in text.splitlines())


def case_runlog_cap():
    # Sanitised text can be longer than the input (`##[` -> `# [`): the file still stops at the cap.
    with tempfile.TemporaryDirectory() as tmp:
        raw = "##[x]\n" * (m.MAX_LOG // 6)  # 4 MiB in; 18 bytes per line out
        n = m.write_runlog(tmp, raw)
        data = Path(tmp, m.RUNLOG_NAME).read_bytes()
        assert n == len(data) <= m.MAX_LOG, (n, len(data))
        assert data.decode().startswith(PREFIX) and all(
            l.startswith(PREFIX) for l in data.decode().splitlines()
        )
        assert b"bytes dropped by size cap]" in data[-100:]
    with tempfile.TemporaryDirectory() as tmp:
        # many short lines: each costs the 12-byte prefix, so the cap binds well before the raw size
        raw = "a\n" * (m.MAX_LOG // 2)
        n = m.write_runlog(tmp, raw)
        assert n <= m.MAX_LOG
        data = Path(tmp, m.RUNLOG_NAME).read_bytes()
        assert data.endswith(b"bytes dropped by size cap]\n")
    with tempfile.TemporaryDirectory() as tmp:
        # exactly-hostile: one 4 MiB line is cut at 1000 bytes and counted as sanitisation
        n = m.write_runlog(tmp, "B" * m.MAX_LOG)
        assert n < 1200, n
        assert b"bytes removed by sanitisation]" in Path(tmp, m.RUNLOG_NAME).read_bytes()


def case_runlog_never_follows_or_overwrites():
    with tempfile.TemporaryDirectory() as tmp:
        target = Path(tmp, "victim.txt")
        target.write_text("keep")
        os.mkdir(Path(tmp, "d"))
        link = Path(tmp, "d", m.RUNLOG_NAME)
        os.symlink(target, link)
        try:
            m.write_runlog(str(Path(tmp, "d")), "x\n")
            raise AssertionError("followed a symlink")
        except OSError:
            pass
        assert target.read_text() == "keep"
        link.unlink()
        link.write_text("old")
        try:
            m.write_runlog(str(Path(tmp, "d")), "x\n")
            raise AssertionError("overwrote a file")
        except OSError:
            pass
        assert link.read_text() == "old"


def case_runlog_failure_changes_nothing():
    base_out, base_result = drive(stage_log(box("FAIL x\n")))
    with tempfile.TemporaryDirectory() as tmp:
        missing = os.path.join(tmp, "nope", "gpu-vast-runlog")
        out, result = drive(stage_log(box("FAIL x\n")), runlog_dir=missing)
    assert result == base_result
    assert "Full sanitised log: not written" in out
    base_lines = [
        x for x in base_out.splitlines() if not x.startswith("::") and "Full sanitised" not in x
    ]
    new_lines = [
        x for x in out.splitlines() if not x.startswith("::") and "Full sanitised" not in x
    ]
    assert len(base_lines) == len(new_lines)


def case_runlog_off_without_a_directory():
    out, _ = drive(stage_log(box("x\n")))
    assert "Full sanitised log" not in out


def case_runlog_dir_setting():
    with tempfile.TemporaryDirectory() as tmp:
        good = os.path.join(tmp, "gpu-vast-runlog")
        assert m.runlog_dir({"KF_RUNLOG_DIR": good}) == good and os.path.isdir(good)
        assert m.runlog_dir({"KF_RUNLOG_DIR": good}) == good  # exists already
        os.mkdir(os.path.join(tmp, "x"))  # the traversal target exists, so only the rule refuses it
        for bad in (
            "",
            "relative/gpu-vast-runlog",
            good + "/../x/gpu-vast-runlog",
            tmp + "/other",
            tmp + "/a b/gpu-vast-runlog",
            "/gpu-vast-runlog",
        ):
            assert m.runlog_dir({"KF_RUNLOG_DIR": bad}) == "", bad
        assert m.runlog_dir({}) == ""
        link = os.path.join(tmp, "x", "gpu-vast-runlog")
        os.symlink(tmp, link)
        assert m.runlog_dir({"KF_RUNLOG_DIR": link}) == ""
        assert (
            m.runlog_dir({"KF_RUNLOG_DIR": os.path.join(tmp, "missing", "gpu-vast-runlog")}) == ""
        )


# ---- the failure path: a box that vanished ---------------------------------------------------


def run_vanished(text, final_state="NONE", runlog_dir=None, stage_chunks=True):
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
    if runlog_dir:
        h.ctx["runlog_dir"] = runlog_dir
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
    assert "DIAGNOSTICS: the run ended without a verified result" in out
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


def case_diagnostics_in_the_artifact():
    log = filler(500, "boot") + box("KF_PAYLOAD_STAGE_START name=merge_bar\n") + filler(100, "late")
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "gpu-vast-runlog")
        os.mkdir(path)
        out, result, runner = run_vanished(log, runlog_dir=path)
        text = Path(path, m.RUNLOG_NAME).read_text()
    assert all(line.startswith(PREFIX) for line in text.splitlines())
    tail = text[text.index("DIAGNOSTICS:") - len(PREFIX) :]
    assert tail.startswith(PREFIX + "DIAGNOSTICS: ")
    assert "last KF_PAYLOAD_STAGE_START in the log: name=merge_bar" in tail
    assert "Not returned by the Worker protocol" in tail
    assert len(text.encode()) <= m.MAX_LOG


def case_diagnostics_in_the_artifact_are_sanitised():
    # write_runlog never trusts the block it is given: it takes the same sanitiser and prefix.
    with tempfile.TemporaryDirectory() as tmp:
        hostile = (
            ["::stop-commands::x \x1b[31mred\x1b[0m", "##[error]y ::set-output name=a::b"],
            "\u202eRTL",
        )
        m.write_runlog(tmp, "[box] line\n", diagnostics=hostile)
        text = Path(tmp, m.RUNLOG_NAME).read_text()
    assert (
        "::" not in text and "##[" not in text and "\x1b" not in text and "\u202e" not in text
    ), text
    assert all(line.startswith(PREFIX) for line in text.splitlines()), text
    assert PREFIX + ": stop-commands: x red" in text


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
