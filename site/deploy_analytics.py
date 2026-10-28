#!/usr/bin/env python3
"""Add environment-provided analytics only to the upstream Pages deployment archive."""

from html.parser import HTMLParser
import io
import os
from pathlib import Path
import sys
import tarfile
import tempfile


def eligible(environment):
    return (
        environment.get("GITHUB_ACTIONS") == "true"
        and environment.get("GITHUB_REPOSITORY") == "reindertpelsma/kayfabe"
        and environment.get("GITHUB_JOB") == "deploy"
        and environment.get("SITE_DEPLOY_ENVIRONMENT") == "github-pages"
        and environment.get("GITHUB_EVENT_NAME")
        in {"push", "release", "workflow_dispatch", "schedule"}
        and (
            environment.get("GITHUB_REF") == "refs/heads/master"
            or environment.get("GITHUB_REF", "").startswith("refs/tags/v")
        )
    )


class Head(HTMLParser):
    def __init__(self, text):
        super().__init__(convert_charrefs=False)
        self.lines = text.splitlines(keepends=True)
        self.start = self.end = None
        self.stack = []
        self.feed(text)
        self.close()
        if self.start is None or self.end is None or self.stack:
            raise ValueError("one complete head required")

    def position_offset(self):
        line, column = self.getpos()
        return sum(map(len, self.lines[: line - 1])) + column

    def handle_starttag(self, tag, attrs):
        if tag == "head":
            if self.start is not None:
                raise ValueError("duplicate head")
            self.start = self.position_offset() + len(self.get_starttag_text())
        if tag == "body" and self.end is None:
            raise ValueError("body before closed head")
        if self.start is not None and self.end is None:
            if tag not in {"meta", "link", "base", "br", "hr", "img", "input", "wbr"}:
                self.stack.append(tag)

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in {"meta", "link", "base", "br", "hr", "img", "input", "wbr"}:
            self.handle_endtag(tag)

    def handle_endtag(self, tag):
        if tag == "head":
            if self.end is not None or self.stack != ["head"]:
                raise ValueError("invalid head close")
            self.end = self.position_offset()
            self.stack.pop()
        elif self.stack:
            if self.stack[-1] != tag:
                raise ValueError("unclosed head element")
            self.stack.pop()


def inject(text, snippet):
    head = Head(text)
    count = text.count(snippet)
    if count:
        start = text.index(snippet)
        if count != 1 or not head.start <= start < start + len(snippet) <= head.end:
            raise ValueError("duplicate or misplaced analytics")
        return text
    result = text[: head.end] + "\n" + snippet + "\n" + text[head.end :]
    Head(result)
    return result


def transform(path, snippet, environment):
    if not eligible(environment) or not snippet.strip():
        return
    updates = []
    with tarfile.open(path) as source:
        for member in source:
            data = source.extractfile(member).read() if member.isfile() else None
            if member.isfile() and member.name.lower().endswith(".html"):
                updated = inject(data.decode("utf-8"), snippet).encode("utf-8")
                if data != updated:
                    updates.append((member.name, updated))
    if not updates:
        return
    changes = dict(updates)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as output:
            temporary = Path(output.name)
            with tarfile.open(path) as source, tarfile.open(fileobj=output, mode="w") as target:
                for member in source:
                    data = source.extractfile(member).read() if member.isfile() else None
                    if member.name in changes:
                        data = changes[member.name]
                        member.size = len(data)
                    target.addfile(member, io.BytesIO(data) if data is not None else None)
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    try:
        transform(Path(sys.argv[1]), os.environ.get("WEB_ANALYTICS_SNIPPET", ""), os.environ)
    except (ValueError, OSError, tarfile.TarError, IndexError):
        raise SystemExit("Analytics deployment archive rejected") from None
