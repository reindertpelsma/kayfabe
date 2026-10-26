"""Head-and-tail display policy of the runner (gate visibility, 2026-10-26).

Measured 2026-10-26 (run 40241597724): the live gate's job log showed only the first ~25 KB of the
box output, because the display budget (65000 bytes, 200 lines) was spent on the first lines. The
policy under test keeps the HEAD (first HEAD_BYTES) and the TAIL (last TAIL_BYTES) of the box log
inside the same total budget, with an explicit "[N bytes dropped from the middle]" marker.

usage: runner_head_tail_fixture.py <case>      (KF_RUNNER_PATH overrides the runner under test)
Every case drives the real Runner.execute() against a fake Worker that serves the log in pages.
"""

import hashlib, importlib.util, io, json, os, re, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "runner", os.environ.get("KF_RUNNER_PATH") or HERE.parent / "runner.py"
)
m = importlib.util.module_from_spec(spec)
sys.modules["runner"] = m
spec.loader.exec_module(m)
scope = {}
exec((HERE / "v5_fixture_data.py").read_text(), scope)
extra_status, availability = scope["extra_status"], scope["availability"]

ctx = {"sha": "a" * 40, "run": "123-1", "nonce": "f" * 64}
manifest = {"protocol_version": 6, "worker_version": "v5", "bundle_sha256": "b" * 64}
m.time.sleep = lambda _: None
PREFIX = len("[untrusted] ")


def row(i, filler="lorem ipsum ", reps=8):
    return "row %05d %s\n" % (i, filler * reps)


def run(text, chunk=4000):
    """Return (job log text, result) for a box log served in pages of `chunk` characters."""
    chunks = [text[i : i + chunk] for i in range(0, len(text), chunk)] or [""]
    chunks = [c for c in chunks if c]
    digest = "0" * 64
    for piece in chunks:
        digest = m.chain(digest, piece)
    status = {
        "state": "GRACE",
        "verdict": "FAIL",
        "run": ctx["run"],
        "id": "900",
        "sha": ctx["sha"],
        "cleanup_pending": True,
        "result_at": 2000,
        "ended_at": None,
        "started_at": 1000,
        "deadline_at": 302000,
        "gpu_model": "RTX 3060",
        "cost_estimate_usd": 0.01,
        "log_cursor": len(chunks),
        "log_tail_sanitised": text[-100:],
        "bytes_dropped": 0,
        "tail_bytes_dropped": 0,
        "runner_nonce": ctx["nonce"],
        "log_sha256": digest,
        **manifest,
    }
    status.update(extra_status("FAIL"))
    printed, summary = [], io.StringIO()

    def client(route, method="GET", body=None):
        head = {"content-type": "application/json"}
        if route.startswith("/availability?"):
            return m.Reply(200, head, json.dumps(availability()).encode())
        if route == "/rent" or route.startswith("/status/"):
            return m.Reply(200, head, json.dumps(status).encode())
        if route.startswith("/logs/"):
            cursor = int(route.rsplit("cursor=", 1)[1])
            page = chunks[cursor : cursor + 4]
            data = {
                "cursor": cursor,
                "next_cursor": cursor + len(page),
                "chunks": page,
                "log_increment_sanitised": "".join(page),
                "log_sha256": "0" * 64,
                "bytes_dropped": 0,
            }
            running = "0" * 64
            for piece in chunks[: cursor + len(page)]:
                running = m.chain(running, piece)
            data["log_sha256"] = running
            return m.Reply(200, head, json.dumps(data, ensure_ascii=False).encode())
        return m.Reply(200, head, b"{}")

    result = m.Runner(ctx, manifest, client, emit=printed.append, summary=summary).execute()
    assert result["verdict"] == "FAIL" and result["verified"] == 1, result
    return "".join(printed), result


def shown_rows(output):
    return [int(x) for x in re.findall(r"^\[untrusted\] row (\d{5})\b", output, re.M)]


def split_rows(output, total):
    """Return (head_count, first_tail_index) of the displayed rows; they must be head + tail."""
    seen = shown_rows(output)
    assert seen == sorted(set(seen)), "rows repeated or out of order"
    head = 0
    while head < len(seen) and seen[head] == head:
        head += 1
    tail = seen[head:]
    assert tail and tail == list(range(tail[0], total)), ("tail is not the log's end", tail[:3])
    return head, tail[0]


def marker(output):
    found = re.findall(r"^\[(\d+) bytes dropped from the middle\]$", output, re.M)
    assert len(found) == 1, ("expected exactly one middle marker", found)
    return int(found[0])


def fits_head(h, rowlen):
    return h <= m.HEAD_LINES and h * (rowlen + PREFIX) <= m.HEAD_BYTES


def case_markers():
    n = 1500
    out, _ = run("".join(row(i) for i in range(n)))
    seen = shown_rows(out)
    assert seen[0] == 0 and seen[-1] == n - 1, (seen[:3], seen[-3:])
    marker(out)
    assert (
        out.index("row 00000")
        < out.index("dropped from the middle")
        < out.index("row %05d" % (n - 1))
    )


def case_count():
    n = 1500
    rowlen = len(row(0))
    out, _ = run("".join(row(i) for i in range(n)))
    head, tail_first = split_rows(out, n)
    assert marker(out) == (tail_first - head) * rowlen, (marker(out), head, tail_first)


def case_head_size():
    n = 1500
    rowlen = len(row(0))
    out, _ = run("".join(row(i) for i in range(n)))
    head, _ = split_rows(out, n)
    assert fits_head(head, rowlen), head
    assert not fits_head(head + 1, rowlen), ("head leaves its allotment unused", head)


def case_tail_size():
    n = 1500
    rowlen = len(row(0))
    out, _ = run("".join(row(i) for i in range(n)))
    _, tail_first = split_rows(out, n)
    count = n - tail_first
    assert count <= m.TAIL_LINES and count * (rowlen + PREFIX) <= m.TAIL_BYTES, count
    more = count + 1
    assert more > m.TAIL_LINES or more * (rowlen + PREFIX) > m.TAIL_BYTES, (
        "tail leaves its allotment unused",
        count,
    )


def case_wide_budget():
    # 900-byte lines: the byte budget binds, not the line budget.
    n = 3000
    out, _ = run("".join(row(i, "wide text ", 90) for i in range(n)), chunk=8000)
    assert len(out.encode()) < 70000, len(out.encode())
    assert sum(x.startswith("[untrusted]") for x in out.splitlines()) <= 200
    head, tail_first = split_rows(out, n)
    assert head >= 20 and n - tail_first >= 12, (head, n - tail_first)


def case_wide_sizes():
    # 900-byte lines: both allotments are exact in bytes (the line budget does not bind).
    n = 3000
    rowlen = len(row(0, "w ", 470))
    out, _ = run("".join(row(i, "w ", 470) for i in range(n)), chunk=8000)
    head, tail_first = split_rows(out, n)
    cost = rowlen + PREFIX
    assert head * cost <= m.HEAD_BYTES < (head + 1) * cost, head
    tail = n - tail_first
    assert tail * cost <= m.TAIL_BYTES < (tail + 1) * cost, tail


def check_ragged(mult, add, mod, chunk, ramp=False):
    # Lines of very different lengths: head and tail are exactly the whole lines that fit.
    n = 1200
    reps = (lambda i: 1 + i * 80 // n) if ramp else (lambda i: 1 + (i * mult + add) % mod)
    rows = [row(i, "abcdef ", reps(i)) for i in range(n)]
    cost = [len(x) + PREFIX for x in rows]
    head, used = [], 0
    while len(head) < m.HEAD_LINES and used + cost[len(head)] <= m.HEAD_BYTES:
        used += cost[len(head)]
        head.append(len(head))
    tail, used = [], 0
    while len(tail) < m.TAIL_LINES and used + cost[n - 1 - len(tail)] <= m.TAIL_BYTES:
        used += cost[n - 1 - len(tail)]
        tail.insert(0, n - 1 - len(tail))
    out, _ = run("".join(rows), chunk=chunk)
    assert shown_rows(out) == head + tail, (len(head), len(tail), shown_rows(out)[:4])
    gap = sum(len(x) for x in rows[len(head) : tail[0]])
    assert marker(out) == gap, (marker(out), gap)


def case_ragged():
    check_ragged(7, 0, 90, 5000)


def case_ragged_again():
    check_ragged(13, 5, 60, 7000)


def case_ragged_ramp():
    check_ragged(0, 0, 1, 6000, ramp=True)


def case_flood_lines():
    # rustc-warning shaped flood: thousands of short lines; the line budget binds.
    n = 20000
    out, _ = run("".join("row %05d w\n" % i for i in range(n)), chunk=12000)
    lines = sum(x.startswith("[untrusted]") for x in out.splitlines())
    assert lines <= 200, lines
    head, tail_first = split_rows(out, n)
    assert head == m.HEAD_LINES and n - tail_first == m.TAIL_LINES, (head, n - tail_first)
    assert marker(out) == (tail_first - head) * len("row 00000 w\n")


def case_scrub():
    text = "\x1b[31mrow 00000 RED\x1b[0m ::error::boom\n"
    text += "".join(row(i) for i in range(1, 1499))
    text += "row 01499 ##[group]‮\x00evil ::set-output name=x::y\n"
    out, _ = run(text)
    for bad in ("\x1b", "::error", "##[", "‮", "\x00", "::set-output"):
        assert bad not in out, bad
    assert "row 00000 RED" in out and "row 01499" in out
    marker(out)
    body = [x for x in out.splitlines() if x.startswith("row ")]
    assert body == [], "unprefixed box text reached the job log"


def case_summary_tail():
    summary = "KF_FINAL_SUMMARY_BEGIN verdict=FAIL\n"
    summary += "".join("stage name=s%02d rc=1 duration_ms=%d\n" % (i, i * 1000) for i in range(30))
    summary += "KF_FINAL_SUMMARY_END\n"
    out, _ = run("".join(row(i) for i in range(2000)) + summary)
    assert out.rindex("KF_FINAL_SUMMARY_END") > out.rindex("stage name=s29")
    assert "KF_FINAL_SUMMARY_BEGIN verdict=FAIL" in out and "stage name=s00" in out
    assert out.index("dropped from the middle") < out.index("KF_FINAL_SUMMARY_BEGIN")


def case_small_log():
    out, _ = run("".join(row(i) for i in range(10)))
    assert "dropped from the middle" not in out
    assert shown_rows(out)[:10] == list(range(10)), "the head shows the whole log"


def case_exact_head():
    n = m.HEAD_LINES
    out, _ = run("".join("row %05d\n" % i for i in range(n)))
    assert "dropped from the middle" not in out
    assert shown_rows(out)[:n] == list(range(n)), "the head shows the whole log"


def case_budget_line_kept():
    out, _ = run("".join(row(i) for i in range(1500)))
    assert re.search(r"^\[\d+ bytes dropped by runner display budget\]$", out, re.M), out[-300:]
    assert out.rstrip().endswith("::endgroup::")


def case_constants():
    assert (m.HEAD_BYTES, m.TAIL_BYTES) == (24000, 16000), (m.HEAD_BYTES, m.TAIL_BYTES)
    assert 0 < m.HEAD_LINES < 199 and 0 < m.TAIL_LINES < 199
    assert m.HEAD_LINES + m.TAIL_LINES <= 170, "leave room for availability and status lines"
    assert m.HEAD_BYTES + m.TAIL_BYTES <= 40000 < 65000


def case_direct_show_budget_unchanged():
    printed = []
    r = m.Runner(ctx, manifest, lambda *_: None, emit=printed.append)
    for _ in range(1000):
        r.show("safe\n" * 10)
    assert len("".join(printed).encode()) < 70000


if __name__ == "__main__":
    globals()["case_" + sys.argv[1].replace("-", "_")]()
