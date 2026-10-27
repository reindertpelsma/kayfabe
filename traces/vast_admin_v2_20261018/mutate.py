"""Local V2 boundary mutants, restored between probes; no provider traffic."""

from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
MAIN = "ci/vast-broker/src/index.js"
UI = "ci/vast-admin/worker.js"
ROUTES = "ci/vast-broker/src/routes.js"
TESTS = ["ci/vast-broker/test/admin-log.test.js", "ci/vast-admin/test/log.test.mjs"]
source = (ROOT / UI).read_text()
escape_start = source.index("      const escape =")
escape_end = source.index("      const html =", escape_start)

# FALSIFIER: any mutant fails fewer than two independent tests or source is not restored.
MUTANTS = [
    (
        "L01 internal origin bypass",
        MAIN,
        "url.origin !== 'https://internal.invalid' || request.method !== 'GET'",
        "request.method !== 'GET'",
    ),
    (
        "L02 public log route exposed",
        ROUTES,
        "export function routeKind(path) {",
        "export function routeKind(path) {\n if(path === '/admin/log') return 'runner';",
    ),
    (
        "L03 record capability disclosure",
        MAIN,
        "text: page.log_increment_sanitised,",
        "...record, text: page.log_increment_sanitised,",
    ),
    (
        "L04 four chunks per page",
        MAIN,
        "ledger.logPage(record, cursor, 1)",
        "ledger.logPage(record, cursor)",
    ),
    (
        "L05 byte limit removed",
        MAIN,
        "encoder.encode(page.log_increment_sanitised).length > 16384",
        "encoder.encode(page.log_increment_sanitised).length > Infinity",
    ),
    (
        "L06 traversal truncated",
        MAIN,
        "next_cursor: page.next_cursor < count ? page.next_cursor : null,",
        "next_cursor: null,",
    ),
    (
        "L07 phantom final page",
        MAIN,
        "next_cursor: page.next_cursor < count ? page.next_cursor : null,",
        "next_cursor: page.next_cursor,",
    ),
    (
        "L08 final summary hidden",
        MAIN,
        "summary: blocks.at(-1)?.[0].replace(/^\\n/, '') ?? '',",
        "summary: '',",
    ),
    (
        "L09 whole tail used as summary",
        MAIN,
        "summary: blocks.at(-1)?.[0].replace(/^\\n/, '') ?? '',",
        "summary: tail,",
    ),
    (
        "L10 missing log reported available",
        MAIN,
        "return json({ text: 'log expired', summary: '', cursor, next_cursor: null }, 404);",
        "return json({ text: 'log expired', summary: '', cursor, next_cursor: null }, 200);",
    ),
    ("L11 cursor range bypass", MAIN, "if (cursor > count)", "if (false)"),
    (
        "L12 HTML escape bypass",
        UI,
        source[escape_start:escape_end],
        "      const escape = (text) => String(text);\n",
    ),
    (
        "L13 Next loses run binding",
        UI,
        "'/log?run=' + run + '&cursor=' + page.next_cursor",
        "'/log?cursor=' + page.next_cursor",
    ),
    (
        "L14 Next on exhausted page",
        UI,
        "Number.isSafeInteger(page.next_cursor) && page.next_cursor > Number(cursor)",
        "true",
    ),
    ("L15 log links omitted", UI, "log.append(link);", ""),
    (
        "L16 Stop requires provider ID",
        UI,
        "if (run.ended_at === null)",
        "if (run.ended_at === null && run.instance_id)",
    ),
    ("L17 terminal rows get Stop", UI, "if (run.ended_at === null)", "if (true)"),
    ("L18 log owner check bypass", UI, "identity.email !== env.OWNER_EMAIL", "false"),
    (
        "L19 log error body disclosed",
        UI,
        "if (!response.ok) return html('<pre>log unavailable</pre>', response.status);",
        "if (!response.ok) return html(await response.text(), response.status);",
    ),
    (
        "L20 missing log loses expiry message",
        UI,
        "if (response.status === 404) return html('<pre>log expired</pre>', 404);",
        "if (response.status === 404) return html('<pre>log unavailable</pre>', 404);",
    ),
]


def probe():
    originals = {name: (ROOT / name).read_text() for _, name, _, _ in MUTANTS}
    results = []
    try:
        for label, name, before, after in MUTANTS:
            original = originals[name]
            if original.count(before) != 1:
                raise RuntimeError(f"non-unique anchor: {label}")
            try:
                (ROOT / name).write_text(original.replace(before, after))
                run = subprocess.run(
                    ["node", "--test", *TESTS],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    timeout=45,
                    check=False,
                )
            finally:
                (ROOT / name).write_text(original)
            kills = re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M)
            if run.returncode == 0 or len(kills) < 2:
                raise RuntimeError(f"{label}: only {len(kills)} kills")
            results.append((label, kills))
            print(f"{label}: {len(kills)} killing tests", flush=True)
    finally:
        for name, text in originals.items():
            (ROOT / name).write_text(text)
    rows = ["| Mutant | Kills | Two witnesses |", "|---|---:|---|"]
    for label, kills in results:
        rows.append(f"| {label} | {len(kills)} | {'; '.join(kills[:2])} |")
    Path(__file__).with_name("mutations.md").write_text("\n".join(rows) + "\n")


if __name__ == "__main__":
    probe()
