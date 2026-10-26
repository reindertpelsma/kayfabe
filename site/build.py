#!/usr/bin/env python3
"""Build every docs version and landing with this checkout's pinned tooling/configs."""

import argparse
import hashlib
from html import escape
from html.parser import HTMLParser
import io
import json
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
    return {"versions": versions, "refs": ["master", *tags], "latest": latest, "root": root}


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
    shutil.copytree(source / "docs/howto", destination)
    summary = (destination / "SUMMARY.md").read_text()
    if (source / "docs/FAQ.md").is_file():
        shutil.copy2(source / "docs/FAQ.md", destination / "FAQ.md")
        summary = summary.rstrip() + "\n\n- [FAQ](FAQ.md)\n"
    (destination / "SUMMARY.md").write_text(summary)
    return re.findall(r"\]\(([^)]+\.md)\)", summary)


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
    return {
        "version": version,
        "manual_pages": len(selected),
        "documents": selected,
    }


def landing_build(source, destination, work, binaries, base_url):
    landing = work / "landing"
    landing.mkdir()
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
        path = unquote(urlsplit(link).path)
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


def metadata(output, plan):
    """Apply one URL policy to both generators and emit the combined sitemap."""
    description = "Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, "
    description += "while the host keeps using it."
    (output / "404.html").write_text(
        '<html lang="en"><head><title>Page not found — kayfabe</title></head>'
        '<body><h1>Page not found</h1><a href="/">Home</a></body></html>'
    )
    sitemap = []
    for page in sorted(output.rglob("*.html")):
        relative = page.relative_to(output)
        text = page.read_text()
        title = check_page(text, relative.as_posix())
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
        # Remove generator metadata before adding this shared policy.
        text = re.sub(
            r'<meta\b[^>]*(?:name=["\'](?:description|robots)["\']'
            r'|property=["\']og:[^"\']+["\'])[^>]*>',
            "",
            text,
            flags=re.I,
        )
        text = re.sub(r'<link\b[^>]*rel=["\']canonical["\'][^>]*>', "", text, flags=re.I)
        tags = '\n<meta name="description" content="' + escape(description, quote=True) + '">'
        for name, value in [
            ("title", title),
            ("description", description),
            ("url", "https://kayfabe.dev" + url),
        ]:
            tags += (
                '\n<meta property="og:' + name + '" content="' + escape(value, quote=True) + '">'
            )
        tags += '\n<link rel="canonical" href="https://kayfabe.dev' + escape(canonical) + '">'
        if noindex:
            tags += '\n<meta name="robots" content="noindex">'
        text = text.replace("</head>", tags + "\n</head>")
        page.write_text(text)
        if not noindex and (
            url in ["/", "/docs/"]
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


def build(repo, binaries, releases, root_ref="latest"):
    plan = version_plan(repo, releases, root_ref)
    output = repo / "site/public"
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    counts = []
    with tempfile.TemporaryDirectory(prefix="kayfabe-site-") as temporary:
        work = Path(temporary)
        landings = {}
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
                landing_build(source, landing, folder, binaries, "https://kayfabe.dev")
                landings[version] = landing
            if version == "master":
                stage = folder / "staging"
                stage.mkdir()
                landing_build(
                    source, output / "staging", stage, binaries, "https://kayfabe.dev/staging"
                )
        shutil.copytree(landings[plan["root"]], output, dirs_exist_ok=True)
        docs = output / "docs"
        docs.mkdir(exist_ok=True)
        target = version_url(plan["latest"])
        index = "<h1>Documentation versions</h1><nav>" + version_links(plan) + "</nav>"
        index += search_form(target)
        index += (
            '<p><a href="https://github.com/reindertpelsma/kayfabe/tree/master/docs/">'
            "Design and internals (GitHub)</a></p>"
        )
        (docs / "index.html").write_text(redirect_page("Documentation versions", target, index))
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
            page.write_text(page.read_text().replace("</main>", search_form(target) + "</main>"))
        metadata(output, plan)
    report = {**plan, "counts": counts, "broken_links": broken_links(output)}
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
    parser.add_argument("--root-ref", choices=["latest", "master"], default="latest")
    args = parser.parse_args()
    releases = json.loads(args.releases_json.read_text()) if args.releases_json else []
    build(SITE.parent, tools(args.tools_dir.resolve()), releases, args.root_ref)


if __name__ == "__main__":
    main()
