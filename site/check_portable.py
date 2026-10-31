"""Crawl a portable build over /x/y HTTP and resolve its file:// resource targets."""

import argparse
from functools import partial
from html.parser import HTMLParser
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import posixpath
from pathlib import Path
import re
import tempfile
import threading
from urllib.error import HTTPError
from urllib.parse import unquote, urljoin, urlsplit
from urllib.request import urlopen


class References(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.urls = []
        self.base = None
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "base":
            self.base = attrs.get("href")
            return
        for name in ("href", "src", "action", "poster", "data"):
            if name in attrs:
                self.urls.append(attrs[name])
        if "srcset" in attrs:
            self.urls.extend(
                re.findall(r"(?:^|,\s+)(\S+)(?:\s+\d[^,]*)?", attrs["srcset"])
            )
        if "style" in attrs:
            self.urls.extend(css_references(attrs["style"]))
        if tag == "meta":
            key = attrs.get("property", attrs.get("name", "")).lower()
            if key in ("og:url", "og:image", "twitter:url", "twitter:image"):
                self.urls.append(attrs.get("content", ""))
            if attrs.get("http-equiv", "").lower() == "refresh":
                value = re.split(
                    r";\s*url\s*=\s*", attrs.get("content", ""), flags=re.I
                )
                if len(value) == 2:
                    self.urls.append(value[1].strip("'\""))

    handle_startendtag = handle_starttag


def css_references(text):
    # This independent checker includes all real generated CSS url() assets.
    return [
        m[1].strip() for m in re.finditer(r"""url\(\s*["']?([^\s"')]+)""", text, re.I)
    ]


def references(output):
    result = []
    for page in sorted(output.rglob("*.html")):
        parser = References(page.read_text())
        result.extend((page, url, parser.base) for url in parser.urls)
        if parser.base:
            result.append((page, parser.base, None))
    for sheet in sorted(output.rglob("*.css")):
        result.extend((sheet, url, None) for url in css_references(sheet.read_text()))
    # mdBook loads its TOC/search through JS; the custom selector loads this JSON.
    for script in sorted((output / "docs").rglob("theme/versions.js")):
        book = script.parent.parent
        json_path = posixpath.relpath(
            "versions.json", script.parent.relative_to(output / "docs").as_posix()
        )
        result.append((script, json_path, None))
        for resource in ("searchindex.js", "toc.js", "toc.html"):
            result.append((book / "index.html", resource, None))
    return result


def check(output):
    output = output.resolve()
    refs = references(output)
    failures = []
    targets = set()
    local_count = 0
    with tempfile.TemporaryDirectory(
        prefix="portable-http-", dir=output.parent
    ) as temp:
        mount = Path(temp) / "x/y"
        mount.parent.mkdir()
        mount.symlink_to(output, target_is_directory=True)

        class Quiet(SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass

        handler = partial(Quiet, directory=temp)
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        prefix = f"http://127.0.0.1:{server.server_port}/x/y/"
        try:
            for source, value, base in refs:
                split = urlsplit(value)
                if split.scheme or split.netloc:
                    if split.netloc == "kayfabe.dev":
                        failures.append(
                            [
                                str(source.relative_to(output)),
                                value,
                                "production absolute URL",
                            ]
                        )
                    continue
                if not value or value.startswith("#"):
                    continue
                local_count += 1
                if value.startswith("/") or split.path.endswith("/"):
                    failures.append(
                        [
                            str(source.relative_to(output)),
                            value,
                            "root absolute or implicit index",
                        ]
                    )
                relative = source.relative_to(output).as_posix()
                http_page = urljoin(prefix, relative)
                http_base = urljoin(http_page, base) if base else http_page
                target = urljoin(http_base, value)
                if not target.startswith(prefix):
                    failures.append([relative, value, "escaped HTTP prefix"])
                targets.add(target.split("#")[0])
                file_base = urljoin(source.as_uri(), base) if base else source.as_uri()
                file_target = Path(unquote(urlsplit(urljoin(file_base, value)).path))
                if not file_target.is_relative_to(output) or not file_target.is_file():
                    failures.append([relative, value, "missing or escaped file target"])
            for target in sorted(targets):
                try:
                    with urlopen(target, timeout=5) as response:
                        response.read()
                        if response.status != 200 or response.url != target:
                            failures.append([target, "", "non-200 or redirect"])
                except Exception as error:
                    failures.append([target, "", str(error)])
                    if isinstance(error, HTTPError):
                        error.close()
        finally:
            server.shutdown()
            worker.join()
            server.server_close()
    return {
        "html_pages": len(list(output.rglob("*.html"))),
        "local_references": local_count,
        "http_targets": len(targets),
        "broken": failures,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "output", nargs="?", type=Path, default=Path(__file__).parent / "public"
    )
    args = parser.parse_args()
    report = check(args.output)
    print(json.dumps(report, indent=2))
    raise SystemExit(bool(report["broken"]))


if __name__ == "__main__":
    main()
