"""The /status/ section: platform test coverage pages (docs/design/V3_PLATFORM_TESTING.md §9).

Written by the master build tooling, not by a ref's Zola templates. Three modes:
- live: an API base (repository variable STATUS_API_BASE, deployment candidates only) that
  passes validate_origin(); the base goes into one <meta> per page, status.js holds no URL;
  coverage.json (evidence imported from traces, scripts/site/coverage_from_traces.py) is a static
  file next to the pages and holds no URL either;
- fixture: --status-fixture serves the live part of the pages from a local synthetic JSON file;
- otherwise one short "not configured" page and nothing else: local, fork and PR builds
  contain no Worker URL.
"""

from html import escape
import json
from pathlib import Path
import re
import shutil

SOURCE = Path(__file__).resolve().parent / "status"
REPO_URL = "https://github.com/reindertpelsma/kayfabe"
ORIGIN = re.compile(r"https://(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)*kayfabe\.dev/?")

PAGES = {
    "matrix": ("status/index.html", "Platform coverage", "Which GPU dies and guests kayfabe's code has been run on, with run, revision and date."),
    "die": ("status/die/index.html", "Die coverage", "Every run recorded for one GPU die: live results and evidence in the repository."),
    "run": ("status/run/index.html", "Test run", "One CI Worker run: host facts, stages and log."),
    "session": ("status/session/index.html", "Host session", "Everything one test box ran in one session: its subruns, results, numbers and log."),
    "commit": ("status/commit/index.html", "Commit results", "The CI Worker runs recorded for one commit or result key."),
    "stats": ("status/stats/index.html", "Results by die and driver", "Latest performance numbers and pass counts for every GPU die and driver, as the CI Worker's runs reported them."),
    "find": ("status/find/index.html", "Find my setup", "Pick a guest OS, a GPU and a CPU vendor and see which driver versions kayfabe has been run on, with result, date and run."),
}

SUBNAV = [
    ("matrix", "/status/", "Matrix"),
    ("stats", "/status/stats/", "Results by die"),
    ("find", "/status/find/", "Find my setup"),
]

NAV = [
    ("/works-today/", "What works today"),
    ("/how-it-differs/", "How it differs"),
    ("/benchmarks/", "Benchmarks"),
    ("/docs/", "Documentation"),
    ("/status/", "Status"),
    ("/about/", "About"),
    ("/security/", "Security"),
    (REPO_URL, "GitHub"),
]


def validate_origin(value):
    """An https origin on kayfabe.dev or a subdomain: no path, credentials, port or query."""
    if not isinstance(value, str) or not ORIGIN.fullmatch(value):
        return None
    return value.rstrip("/")


def shell(page, title, description, body, docs_url, meta, assets=True):
    links = "".join(
        '<li><a href="{}"{}>{}</a></li>'.format(
            escape(href, quote=True),
            ' aria-current="page"' if href == "/status/" else "",
            escape(label),
        )
        for href, label in NAV
    )
    metas = "".join(
        '\n    <meta name="{}" content="{}" />'.format(escape(k, quote=True), escape(v, quote=True))
        for k, v in meta.items()
    )
    assets_html = (
        '    <link rel="stylesheet" href="/status/status.css" />\n'
        '    <script src="/status/status.js" defer></script>'
        if assets
        else ""
    )
    return f"""<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light dark" />
    <title>{escape(title)} — kayfabe</title>
    <meta name="description" content="{escape(description, quote=True)}" />{metas}
    <link rel="icon" href="/icon.svg" type="image/svg+xml" />
    <link rel="stylesheet" href="/style.css" />
    <script src="/site.js" defer></script>
{assets_html}
  </head>
  <body>
    <a class="skip-link" href="#main">Skip to content</a>
    <header class="site-header">
      <div class="wrap bar">
        <a class="brand" href="/">kayfabe</a>
        <details class="menu" open>
          <summary>Menu</summary>
          <nav aria-label="Site"><ul>{links}</ul></nav>
        </details>
      </div>
    </header>
    <main id="main" tabindex="-1" class="wrap kf-status" data-page="{page}">
{body}
    </main>
    <footer class="site-footer" id="kf-footer">
      <div class="wrap">
        <ul class="footer-links">
          <li><a href="{REPO_URL}/blob/master/LICENSE">Licence: Apache-2.0 OR GPL-2.0-or-later</a></li>
          <li><a href="{escape(docs_url, quote=True)}credits.html">Credits and notices</a></li>
          <li><a href="{REPO_URL}">GitHub</a></li>
          <li><a href="/about/">About</a></li>
        </ul>
        <p>Kayfabe is not affiliated with or endorsed by NVIDIA or Microsoft.</p>
      </div>
    </footer>
  </body>
</html>
"""


def page_body(page, title, fixture):
    banner = (
        '      <p class="kf-synthetic" role="note"><strong>Synthetic data.</strong> '
        "The live results on these pages come from a development fixture; no run in it was measured. "
        "The imported evidence comes from the repository.</p>\n"
        if fixture
        else ""
    )
    intro = {
        "matrix": "What kayfabe's code has been run on: GPU family, die and guest platform, with run, revision and date on every entry.",
        "die": "Every result recorded for one GPU die, by tier and platform, and every imported run.",
        "run": "One run of the CI Worker.",
        "commit": "The CI Worker runs recorded for one commit or result key.",
        "session": "One test box, one session: the host, Linux and Windows subruns it ran, with their results.",
        "stats": "The newest report of each GPU die and driver: what passed, and the performance numbers the box measured. Numbers are measurements by the test box, never a gate result.",
        "find": "Choose your guest OS and GPU to see which driver versions have been run, how the runs ended and when. Every choice is in the address, so a filtered view can be shared.",
    }[page]
    subnav = "".join(
        '<li><a href="{}"{}>{}</a></li>'.format(
            href, ' aria-current="page"' if key == page else "", label
        )
        for key, href, label in SUBNAV
    )
    subnav = f'      <nav class="kf-subnav" aria-label="Status pages"><ul>{subnav}</ul></nav>\n'
    return (
        f"{subnav}      <h1>{escape(title)}</h1>\n{banner}"
        f"      <p>{escape(intro)}</p>\n"
        '      <noscript><p>This page needs JavaScript to load the test results.</p></noscript>\n'
        '      <div id="kf-status-filters"></div>\n'
        '      <p id="kf-status-state" role="status" aria-live="polite">Loading.</p>\n'
        '      <div id="kf-status-content"></div>'
    )


def not_configured(output, docs_url):
    body = (
        "      <h1>Platform coverage</h1>\n"
        "      <p>The status pages are not configured in this build.</p>\n"
        "      <p>Published builds show which GPU dies and drivers pass the tests.</p>"
    )
    target = output / "status/index.html"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(
        shell(
            "not-configured",
            "Platform coverage",
            "The status pages are not configured in this build.",
            body,
            docs_url,
            {"kf-status-mode": "off"},
            assets=False,
        )
    )
    return "off"


def write_status(output, api_base=None, logs_base=None, fixture=None, docs_url="/docs/"):
    """Write /status/. Returns the mode: 'live', 'fixture' or 'off'."""
    output = Path(output)
    if (output / "status").exists():
        shutil.rmtree(output / "status")
    api = validate_origin(api_base) if api_base else None
    if api_base and not api:
        print("::warning::STATUS_API_BASE is not an https origin on kayfabe.dev; status off")
    if fixture is not None and api:
        raise ValueError("--status-fixture and a status API base are exclusive")
    if fixture is None and not api:
        return not_configured(output, docs_url)
    meta = {"kf-status-repo": REPO_URL}
    if fixture is not None:
        data = json.loads(Path(fixture).read_text())
        if data.get("synthetic") is not True or not isinstance(data.get("responses"), dict):
            raise ValueError("status fixture must be a synthetic {responses} file")
        meta["kf-status-mode"] = "fixture"
        meta["kf-status-fixture"] = "fixture.json"
        logs = validate_origin(logs_base) if logs_base else None
        if logs:
            meta["kf-status-logs"] = logs
    else:
        meta["kf-status-mode"] = "live"
        meta["kf-status-api"] = api
        logs = validate_origin(logs_base) if logs_base else None
        if logs:
            meta["kf-status-logs"] = logs
    for page, (path, title, description) in PAGES.items():
        target = output / path
        target.parent.mkdir(parents=True, exist_ok=True)
        body = page_body(page, title, fixture is not None)
        target.write_text(shell(page, title, description, body, docs_url, meta))
    for name in ["status.js", "status.css", "coverage.json", "gpu_names.json"]:
        shutil.copy2(SOURCE / name, output / "status" / name)
    if fixture is not None:
        shutil.copy2(fixture, output / "status/fixture.json")
    return meta["kf-status-mode"]
