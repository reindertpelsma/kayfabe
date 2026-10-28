#!/usr/bin/env python3
"""Build every docs version and landing with this checkout's pinned tooling/configs."""

import argparse
import claims_guard
import hashlib
from html import escape, unescape
from html.parser import HTMLParser
import io
import json
import os
from deployment_slots import candidate_context, prepare_output
from pathlib import Path
import re
import posixpath
import shutil
import subprocess
import tarfile
import tempfile
from urllib.parse import quote, unquote, urlsplit

SITE = Path(__file__).resolve().parent


def git(repo, *args):
    return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()


def has_site(repo, ref):
    return bool(git(repo, "ls-tree", "--name-only", ref, "--", "site"))


REPO_URL = "https://github.com/reindertpelsma/kayfabe"


def size_text(size):
    value = float(size)
    for unit in ["B", "KiB", "MiB", "GiB"]:
        if value < 1024 or unit == "GiB":
            return f"{value:.0f} {unit}" if unit == "B" else f"{value:.1f} {unit}"
        value /= 1024


def release_summary(release):
    """The few fields the landing shows; asset links must stay inside this repository."""
    assets = []
    for asset in release.get("assets", []):
        url = asset.get("browser_download_url", "")
        if not url.startswith(REPO_URL + "/releases/download/") or not asset.get("name"):
            continue
        digest = asset.get("digest") or ""
        assets.append(
            {
                "name": asset["name"],
                "url": url,
                "size": size_text(asset.get("size") or 0),
                "sha256": digest[len("sha256:") :] if digest.startswith("sha256:") else "",
            }
        )
    tag = release["tag_name"]
    return {
        "tag": tag,
        "name": release.get("name") or tag,
        "url": REPO_URL + "/releases/tag/" + quote(tag),
        "date": release["published_at"][:10],
        "prerelease": bool(release.get("prerelease")),
        "assets": assets,
    }


def version_plan(repo, releases, root_ref="latest"):
    tags = git(repo, "tag", "--list", "v*", "--sort=-version:refname").splitlines()
    # --paginate --slurp produces pages; a single page is also accepted locally.
    if releases and isinstance(releases[0], list):
        releases = [release for page in releases for release in page]
    published = [
        r
        for r in releases
        if not r.get("draft")
        and r.get("published_at")
        and r.get("assets")
        and r.get("tag_name", "").startswith("v")
    ]
    for release in published:
        if release["tag_name"] not in tags:
            raise ValueError("Release tag not fetched: " + release["tag_name"])
    published.sort(key=lambda r: (r["published_at"], -tags.index(r["tag_name"])), reverse=True)
    versions = ["master"] if (repo / "docs/howto/SUMMARY.md").is_file() else []
    versions += [
        tag
        for tag in tags
        if git(repo, "ls-tree", "--name-only", tag, "--", "docs/howto/SUMMARY.md")
    ]
    latest = next((r["tag_name"] for r in published if r["tag_name"] in versions), "master")
    root = next((r["tag_name"] for r in published if has_site(repo, r["tag_name"])), "master")
    if root_ref == "master":
        root = "master"
    elif root_ref != "latest":
        raise ValueError("root_ref must be latest or master")
    return {
        "versions": versions,
        "refs": ["master", *tags],
        "latest": latest,
        "root": root,
        "release": release_summary(published[0]) if published else None,
    }


def snapshot(repo, ref, destination):
    destination.mkdir(parents=True, exist_ok=True)
    paths = [
        p
        for p in ["docs/howto", "docs/FAQ.md", "site/content", "site/templates", "site/static"]
        if git(repo, "ls-tree", "--name-only", ref, "--", p)
    ]
    if paths:
        data = subprocess.check_output(["git", "archive", ref, "--", *paths], cwd=repo)
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            archive.extractall(destination, filter="data")


def manual_source(source, destination):
    """Keep the author-written mdBook order; publish only the manual and FAQ."""
    manual = source / "docs/howto"
    shutil.copytree(manual, destination)
    published = {
        page.resolve(): page.relative_to(manual).as_posix()
        for page in manual.rglob("*")
        if page.is_file()
    }
    summary = (destination / "SUMMARY.md").read_text()
    if (source / "docs/FAQ.md").is_file():
        published[(source / "docs/FAQ.md").resolve()] = "FAQ.md"
        summary = summary.rstrip() + "\n\n- [FAQ](FAQ.md)\n"
    pattern = (
        r"(?P<code>(?P<ticks>`+)(?!`)(.*?)(?P=ticks)(?!`))|"
        r"(?P<prefix>!?\[[^\]\n]*\]\()(?P<target>[^)\s]+)(?P<suffix>\))"
    )
    for page, output in published.items():
        if page.suffix != ".md":
            continue

        def rewrite(match):
            if match["code"] is not None:
                return match[0]
            target = urlsplit(match["target"])
            if target.scheme or target.netloc or not target.path:
                return match[0]
            mapped = published.get((page.parent / unquote(target.path)).resolve())
            if mapped is None:
                return match[0]
            relative = posixpath.relpath(mapped, posixpath.dirname(output) or ".")
            target = target._replace(path=quote(relative))
            return match["prefix"] + target.geturl() + match["suffix"]

        (destination / output).write_text(re.sub(pattern, rewrite, page.read_text(), flags=re.S))
    (destination / "SUMMARY.md").write_text(summary)
    return re.findall(r"\]\(([^)]+\.md)\)", summary)


def page_href(file):
    """mdBook turns chapter.md into chapter.html (README.md and index.md into index.html)."""
    return re.sub(r"\.md$", ".html", file)


def parse_summary(text):
    """Sections in SUMMARY.md order: lead-in chapters are 'Start here', each `# Part` is a
    section as written, and the appended FAQ is its own section."""
    sections = [{"title": "Start here", "pages": []}]
    faq = []
    for line in text.splitlines():
        part = re.match(r"#\s+(.+?)\s*$", line)
        if part and part[1] != "Summary":
            sections.append({"title": part[1], "pages": []})
            continue
        chapter = re.match(r"\s*(?:[-*]\s+)?\[([^\]]+)\]\(([^)\s]+\.md)\)", line)
        if chapter:
            page = {"title": chapter[1], "file": chapter[2], "href": page_href(chapter[2])}
            (faq if chapter[2] == "FAQ.md" else sections[-1]["pages"]).append(page)
    if faq:
        sections.append({"title": "FAQ", "pages": faq})
    return [section for section in sections if section["pages"]]


def page_descriptions(index_text):
    """The manual's own 'Pages' table: file -> what the page answers."""
    found = {}
    row = r"^\|\s*\[[^\]]+\]\(([^)]+\.md)\)\s*\|\s*(.+?)\s*\|\s*$"
    for match in re.finditer(row, index_text, re.M):
        found[match[1]] = match[2].replace("`", "")
    return found


def manual_outline(repo, ref):
    """Docs-index sections for one ref, read without checking it out."""

    def read(path):
        if ref == "master":
            return (repo / path).read_text() if (repo / path).is_file() else ""
        try:
            return git(repo, "show", ref + ":" + path)
        except subprocess.CalledProcessError:
            return ""

    summary = read("docs/howto/SUMMARY.md")
    if read("docs/FAQ.md"):
        summary = summary.rstrip() + "\n\n- [FAQ](FAQ.md)\n"
    descriptions = page_descriptions(read("docs/howto/index.md"))
    sections = parse_summary(summary)
    for section in sections:
        for page in section["pages"]:
            page["description"] = descriptions.get(page["file"], "")
    return sections


LABEL_START = re.compile(r"\[(measured|inferred)\b", re.I)
GAP = re.compile(r"\b(not documented yet|not verified end to end)\b", re.I)
BLOCKS = {"p", "li", "td", "th", "div", "ul", "ol", "h1", "h2", "h3", "h4", "table", "pre"}


def mark_evidence(html):
    """Make the manual's evidence labels and visible gaps styleable, text nodes only."""
    out, skip, label = [], 0, None
    for part in re.split(r"(<[^>]*>)", html):
        if part.startswith("<"):
            tag = re.match(r"<(/?)([A-Za-z0-9]+)", part)
            if tag:
                name, closing = tag[2].lower(), bool(tag[1])
                if name in ("pre", "script", "style"):
                    skip = max(0, skip + (-1 if closing else 1))
                if label and name in BLOCKS:
                    out.append("</span>")
                    label = None
            out.append(part)
            continue
        if skip:
            out.append(part)
            continue
        position = 0
        while position < len(part):
            if label:
                end = part.find("]", position)
                if end < 0:
                    out.append(part[position:])
                    break
                out.append(part[position : end + 1] + "</span>")
                position, label = end + 1, None
                continue
            start = LABEL_START.search(part, position)
            chunk = part[position : start.start() if start else len(part)]
            out.append(GAP.sub(r'<span class="kf-gap">\1</span>', chunk))
            if not start:
                break
            label = start[1].lower()
            out.append('<span class="kf-label kf-' + label + '">[')
            position = start.start() + 1
    if label:
        out.append("</span>")
    return "".join(out)


def decorate_chapter(html, sections, file, version_text):
    """Skip link, breadcrumbs, edit link and previous/next text links for one chapter."""
    flat = [(section["title"], page) for section in sections for page in section["pages"]]
    index = next((i for i, (_, page) in enumerate(flat) if page["file"] == file), None)
    if index is None or "<main>" not in html or "</main>" not in html:
        return html
    title, page = flat[index]
    source = "docs/FAQ.md" if file == "FAQ.md" else "docs/howto/" + file
    crumbs = (
        '<nav class="kf-crumbs" aria-label="Breadcrumb"><ol>'
        '<li><a href="/docs/">Docs</a></li>'
        "<li>" + escape(version_text) + "</li>"
        "<li>" + escape(title) + "</li>"
        '<li aria-current="page">' + escape(page["title"]) + "</li></ol></nav>"
    )
    pager = '<nav class="kf-pager" aria-label="Previous and next page">'
    for offset, rel, word in [(-1, "prev", "Previous"), (1, "next", "Next")]:
        if 0 <= index + offset < len(flat):
            other = flat[index + offset][1]
            pager += (
                '<a rel="{}" href="{}"><span>{}</span> {}</a>'.format(
                    rel, escape(other["href"], quote=True), word, escape(other["title"])
                )
            )
    pager += "</nav>"
    edit = (
        '<p class="kf-edit"><a href="'
        + REPO_URL
        + "/edit/master/"
        + source
        + '">View and edit source on GitHub</a> <span>(source: <code>'
        + source
        + "</code>, master branch)</span></p>"
    )
    head, rest = html.split("<main>", 1)
    body, tail = rest.split("</main>", 1)
    body = mark_evidence(body).replace("<p>Verified on:", '<p class="kf-verified">Verified on:')
    skip_link = r'\1<a class="skip-link" href="#content">Skip to content</a>'
    head = re.sub(r"(<body[^>]*>)", skip_link, head, count=1)
    return head + "<main>" + crumbs + body + edit + pager + "</main>" + tail


def tools(directory):
    """Cache verified release archives in a caller-owned temporary directory."""
    directory.mkdir(parents=True, exist_ok=True)
    result = {}
    for name, pin in json.loads((SITE / "tools.json").read_text()).items():
        archive = directory / (name + "-" + pin["version"] + ".tar.gz")
        if not archive.exists():
            subprocess.run(
                [
                    "curl",
                    "--fail",
                    "--location",
                    "--retry",
                    "3",
                    "--silent",
                    "--show-error",
                    "--output",
                    str(archive),
                    pin["url"],
                ],
                check=True,
            )
        if hashlib.sha256(archive.read_bytes()).hexdigest() != pin["sha256"]:
            raise ValueError("SHA-256 mismatch: " + name)
        target = directory / (name + "-" + pin["version"])
        target.mkdir(exist_ok=True)
        with tarfile.open(archive) as bundle:
            member = next(m for m in bundle.getmembers() if Path(m.name).name == name)
            binary = target / name
            binary.write_bytes(bundle.extractfile(member).read())
            binary.chmod(0o755)
        result[name] = str(binary)
    return result


def version_url(version):
    return "/docs/" + quote(version, safe="/") + "/"


def version_links(plan, current=None):
    links = []
    for version in plan["versions"]:
        label = version_label(version, plan)
        if version == current:
            links.append("<strong>" + escape(label) + "</strong>")
        else:
            links.append('<a href="' + version_url(version) + '">' + escape(label) + "</a>")
    return " · ".join(links)


def docs_build(source, version, plan, work, output, binaries):
    book = work / "book"
    book.mkdir()
    selected = manual_source(source, book / "src")
    shutil.copy2(SITE / "book.toml", book / "book.toml")
    shutil.copytree(SITE / "book-theme", book / "theme")
    header = book / "theme/header.hbs"
    header.write_text(
        header.read_text()
        .replace("@@VERSIONS@@", version_links(plan, version))
        .replace("@@CURRENT@@", escape(version, quote=True))
    )
    destination = output / "docs" / version
    subprocess.run(
        [binaries["mdbook"], "build", str(book), "--dest-dir", str(destination)], check=True
    )
    # mdBook's generated no-JS sidebar frame has no title in this pinned release.
    toc = destination / "toc.html"
    toc.write_text(toc.read_text().replace("<head>", "<head><title>Contents — kayfabe</title>"))
    sections = parse_summary((book / "src/SUMMARY.md").read_text())
    for file in selected:
        chapter = destination / page_href(file)
        if chapter.is_file():
            text = decorate_chapter(chapter.read_text(), sections, file, version_label(version, plan))
            chapter.write_text(text)
    return {
        "version": version,
        "manual_pages": len(selected),
        "documents": selected,
    }


def site_data(plan, repo):
    """Build-time facts for the templates: docs sections, versions and the latest release."""
    latest = plan["latest"]
    return {
        "latest": latest,
        "docs_url": version_url(latest),
        "versions": [
            {
                "version": v,
                "label": version_label(v, plan),
                "url": version_url(v),
                "latest": v != "master" and v == latest,
            }
            for v in plan["versions"]
        ],
        "sections": manual_outline(repo, latest) if latest in plan["versions"] else [],
        "release": plan.get("release"),
    }


def landing_build(source, destination, work, binaries, base_url, data=None):
    landing = work / "landing"
    landing.mkdir()
    (landing / "data").mkdir()
    (landing / "data/site.json").write_text(json.dumps(data or {}, indent=2) + "\n")
    shutil.copy2(SITE / "config.toml", landing / "config.toml")
    for folder in ["content", "templates", "static"]:
        original = source / "site" / folder
        if original.exists():
            shutil.copytree(original, landing / folder)
    subprocess.run(
        [
            binaries["zola"],
            "--root",
            str(landing),
            "build",
            "--base-url",
            base_url,
            "--output-dir",
            str(destination),
        ],
        check=True,
    )
    if not (destination / "index.html").is_file():
        raise ValueError("Landing rendered without an index: " + str(source))


class Links(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = set()

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        for key in ["id", "name"]:
            if values.get(key):
                self.ids.add(values[key])
        for key in ["href", "src"]:
            if values.get(key):
                self.links.add(values[key])


def broken_links(output):
    pages = {}
    for page in output.rglob("*.html"):
        parser = Links()
        parser.feed(page.read_text(encoding="utf-8"))
        pages[page.resolve()] = parser
    broken = []
    for page, parser in sorted(pages.items()):
        for link in sorted(parser.links):
            url = urlsplit(link)
            if url.scheme and url.scheme not in ["http", "https"]:
                continue
            if url.netloc and url.netloc != "kayfabe.dev":
                continue
            path = unquote(url.path)
            target = (
                (
                    (output / path.lstrip("/")) if path.startswith("/") else page.parent / path
                ).resolve()
                if path
                else page
            )
            if target.is_dir():
                target /= "index.html"
            reason = None
            if not target.is_file():
                reason = "file missing"
            elif (
                url.fragment and target in pages and unquote(url.fragment) not in pages[target].ids
            ):
                reason = "anchor missing"
            if reason:
                broken.append(
                    {
                        "page": page.relative_to(output.resolve()).as_posix(),
                        "target": link,
                        "reason": reason,
                    }
                )
    return broken


def version_label(version, plan):
    if version == "master":
        return "master (unreleased)"
    return version + (" (latest)" if version == plan["latest"] else "")


def search_form(target):
    return (
        '<form action="' + target + '" method="get"><label>Search docs '
        '<input name="search" type="search"></label><button>Search</button></form>'
    )


def redirect_page(title, target, body=""):
    return (
        '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>'
        + title
        + '</title><meta http-equiv="refresh" content="0;url='
        + target
        + '"></head><body>'
        + body
        + '<p><a href="'
        + target
        + '">Read the latest documentation</a></p></body></html>'
    )


def check_page(text, page):
    title = re.search(r"<title>(.*?)</title>", text, re.I | re.S)
    if not title or not title[1].strip():
        raise ValueError("Published page has no title: " + page)
    links = Links()
    links.feed(text)
    for link in links.links:
        url = urlsplit(link)
        if url.netloc and url.netloc != "kayfabe.dev":
            if url.netloc != "github.com" or not url.path.startswith("/reindertpelsma/kayfabe/"):
                continue
        path = unquote(url.path)
        path = posixpath.normpath(posixpath.join("/" + page, "..", path))
        internal = r"/(design|archive)(/|$)|/(OWNER_RULINGS|STATUS_[^/]*|handoff[^/]*)"
        outside_manual = False
        if "/docs/" in path:
            relative = path.split("/docs/", 1)[1]
            first = relative.split("/", 1)[0]
            outside_manual = bool(relative) and not (
                first in ["master", "latest", "howto", "FAQ.md"] or first.startswith("v")
            )
        if outside_manual or re.search(internal, path, re.I):
            raise ValueError("Internal documentation link in " + page + ": " + link)
    return title[1].strip()


def page_url(path):
    path = path.as_posix()
    return "/" + (path[:-10] if path.endswith("index.html") else path)


def check_public_copy(text, page):
    """Keep the detailed manual outside the main-site prose rules."""
    if str(page).startswith("docs/"):
        return None
    return claims_guard.enforce_page(text, str(page))


def metadata(output, plan):
    """Apply one URL policy to both generators and emit the combined sitemap."""
    description = "Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, "
    description += "while the host keeps using it."
    if not (output / "404.html").is_file():
        (output / "404.html").write_text(
            '<html lang="en"><head><title>Page not found — kayfabe</title></head>'
            '<body><h1>Page not found</h1><a href="/">Home</a></body></html>'
        )
    sitemap = []
    copy_reviews = {}
    for page in sorted(output.rglob("*.html")):
        relative = page.relative_to(output)
        text = page.read_text()
        title = check_page(text, relative.as_posix())
        review = check_public_copy(text, relative.as_posix())
        if review is not None:
            copy_reviews[relative.as_posix()] = review
        if "</head>" not in text.lower():
            body = re.sub(r"<title>.*?</title>", "", text, flags=re.I | re.S)
            text = (
                '<html lang="en"><head><title>'
                + title
                + "</title></head><body>"
                + body
                + "</body></html>"
            )
        url = page_url(relative)
        canonical = url
        if url.startswith("/docs/latest/"):
            canonical = version_url(plan["latest"])
        for version in plan["versions"]:
            prefix = version_url(version)
            if url.startswith(prefix):
                canonical = version_url(plan["latest"]) + url[len(prefix) :]
                break
        if url.startswith("/staging/"):
            canonical = "/" + url[len("/staging/") :]
        noindex = url.startswith(("/staging/", "/docs/master/")) or url.endswith("404.html")
        # A page may carry its own plain description; otherwise the shared one applies.
        own = re.search(
            r'<meta\b[^>]*name=["\']description["\'][^>]*content="([^"]*)"', text, re.I
        )
        summary = unescape(own[1]).strip() if own and own[1].strip() else description
        # Remove generator metadata before adding this shared policy.
        text = re.sub(
            r'<meta\b[^>]*(?:name=["\'](?:description|robots|twitter:[^"\']+)["\']'
            r'|property=["\']og:[^"\']+["\'])[^>]*>',
            "",
            text,
            flags=re.I,
        )
        text = re.sub(r'<link\b[^>]*rel=["\']canonical["\'][^>]*>', "", text, flags=re.I)
        tags = '\n<meta name="description" content="' + escape(summary, quote=True) + '">'
        # Plain text cards: no generated image, so the small "summary" card type.
        for name, value in [
            ("type", "website"),
            ("site_name", "kayfabe"),
            ("title", title),
            ("description", summary),
            ("url", "https://kayfabe.dev" + url),
        ]:
            tags += (
                '\n<meta property="og:' + name + '" content="' + escape(value, quote=True) + '">'
            )
        for name, value in [("card", "summary"), ("title", title), ("description", summary)]:
            tags += (
                '\n<meta name="twitter:' + name + '" content="' + escape(value, quote=True) + '">'
            )
        tags += '\n<link rel="canonical" href="https://kayfabe.dev' + escape(canonical) + '">'
        if noindex:
            tags += '\n<meta name="robots" content="noindex">'
        text = text.replace("</head>", tags + "\n</head>")
        page.write_text(text)
        if not noindex and (
            url in ["/", "/docs/", "/about/", "/security/", "/works-today/",
                    "/how-it-differs/", "/benchmarks/"]
            or (plan["latest"] != "master" and url.startswith(version_url(plan["latest"])))
        ):
            sitemap.append("https://kayfabe.dev" + url)
    (output / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
        + "".join("<url><loc>" + escape(url) + "</loc></url>\n" for url in sitemap)
        + "</urlset>\n"
    )
    (output / "robots.txt").write_text("Sitemap: https://kayfabe.dev/sitemap.xml\n")
    return copy_reviews


def build(repo, binaries, releases, root_ref="latest", deploy_candidate=False):
    plan = version_plan(repo, releases, root_ref)
    output = repo / "site/public"
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    counts = []
    with tempfile.TemporaryDirectory(prefix="kayfabe-site-") as temporary:
        work = Path(temporary)
        landings = {}
        data = site_data(plan, repo)
        data["deploy_candidate"] = deploy_candidate
        for number, version in enumerate(plan["refs"]):
            folder = work / str(number)
            folder.mkdir()
            source = repo
            if version != "master":
                source = folder / "source"
                snapshot(repo, version, source)
            if version in plan["versions"]:
                counts.append(docs_build(source, version, plan, folder, output, binaries))
            if (source / "site").is_dir():
                landing = folder / "rendered"
                landing_build(source, landing, folder, binaries, "https://kayfabe.dev", data)
                landings[version] = landing
            if version == "master":
                stage = folder / "staging"
                stage.mkdir()
                landing_build(
                    source,
                    output / "staging",
                    stage,
                    binaries,
                    "https://kayfabe.dev/staging",
                    data,
                )
        shutil.copytree(landings[plan["root"]], output, dirs_exist_ok=True)
        docs = output / "docs"
        docs.mkdir(exist_ok=True)
        target = version_url(plan["latest"])
        # A landing without the docs-index template (an older tag) keeps the plain version list.
        current = docs / "index.html"
        if not current.is_file() or 'data-kf="docs-index"' not in current.read_text():
            index = "<h1>Documentation versions</h1><nav>" + version_links(plan) + "</nav>"
            index += search_form(target)
            index += (
                '<p><a href="https://github.com/reindertpelsma/kayfabe/tree/master/docs/">'
                "Design and internals (GitHub)</a></p>"
            )
            current.write_text(redirect_page("Documentation versions", target, index))
        latest = docs / "latest"
        latest.mkdir(exist_ok=True)
        (latest / "index.html").write_text(redirect_page("Latest documentation", target))
        (docs / "versions.json").write_text(
            json.dumps(
                [
                    {
                        "version": v,
                        "label": version_label(v, plan),
                        "latest": v != "master" and v == plan["latest"],
                    }
                    for v in plan["versions"]
                ],
                indent=2,
            )
            + "\n"
        )
        for page in [output / "index.html", output / "staging/index.html"]:
            text = page.read_text()
            if 'name="search"' not in text:
                page.write_text(text.replace("</main>", search_form(target) + "</main>"))
        copy_reviews = metadata(output, plan)
        prepare_output(output, deploy_candidate)
    report = {
        **plan,
        "counts": counts,
        "broken_links": broken_links(output),
        "copy_review": copy_reviews,
    }
    (repo / "site/build-report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                "root": plan["root"],
                "latest_docs": plan["latest"],
                "versions": len(counts),
                "manual_pages": sum(c["manual_pages"] for c in counts),
                "broken_internal_links": len(report["broken_links"]),
            },
            indent=2,
        )
    )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-dir", type=Path, required=True)
    parser.add_argument("--releases-json", type=Path)
    parser.add_argument("--deploy-candidate", action="store_true")
    parser.add_argument("--root-ref", choices=["latest", "master"], default="latest")
    args = parser.parse_args()
    releases = json.loads(args.releases_json.read_text()) if args.releases_json else []
    build(
        SITE.parent,
        tools(args.tools_dir.resolve()),
        releases,
        args.root_ref,
        deploy_candidate=args.deploy_candidate and candidate_context(os.environ),
    )


if __name__ == "__main__":
    main()
