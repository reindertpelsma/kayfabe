"""Opt-in portable URLs: edit URL value spans, never serialize an HTML document."""

from html import escape, unescape
from html.parser import HTMLParser
from pathlib import Path
import posixpath
import re
from urllib.parse import quote, unquote, urlsplit, urlunsplit


URL_ATTRIBUTES = {"href", "src", "action", "poster", "data"}
META_URLS = {"og:url", "og:image", "twitter:url", "twitter:image"}
ATTRIBUTE = re.compile(r"""([^\s/>=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?""")


def relative_url(value, page, output):
    """Resolve a site URL against its source page, retaining query and fragment."""
    if not value or value.startswith("#"):
        return value
    url = urlsplit(value)
    if url.scheme or url.netloc:
        if url.scheme not in ("", "http", "https") or url.netloc != "kayfabe.dev":
            return value
    path = unquote(url.path)
    source = page.relative_to(output).as_posix()
    target = (
        posixpath.normpath(
            path.lstrip("/")
            if path.startswith("/")
            else posixpath.join(posixpath.dirname(source), path)
        )
        if path
        else source
    )
    if target == ".." or target.startswith("../"):
        raise ValueError("Portable URL escapes output: " + value)
    if path.endswith("/") or (output / target).is_dir():
        target = posixpath.join(target, "index.html")
    relative = posixpath.relpath(target, posixpath.dirname(source) or ".")
    return urlunsplit(
        ("", "", quote(relative, safe="/@:+,;=-._~!$&'()*"), url.query, url.fragment)
    )


def srcset_urls(value, convert):
    """Use srcset's non-space URL tokens; data URL commas stay inside the token."""
    edits = []
    cursor = 0
    while cursor < len(value):
        while cursor < len(value) and (value[cursor].isspace() or value[cursor] == ","):
            cursor += 1
        start = cursor
        while cursor < len(value) and not value[cursor].isspace():
            cursor += 1
        end = cursor
        while end > start and value[end - 1] == ",":
            end -= 1
        if end > start:
            edits.append((start, end, convert(value[start:end])))
        if end != cursor:
            continue
        # Descriptor tokens end at the next separator (parentheses may contain commas).
        depth = 0
        while cursor < len(value):
            char = value[cursor]
            cursor += 1
            depth += (char == "(") - (char == ")")
            if char == "," and depth == 0:
                break
    return apply_edits(value, edits)


def apply_edits(text, edits):
    for start, end, replacement in sorted(edits, reverse=True):
        text = text[:start] + replacement + text[end:]
    return text


def html_urls(text, page, output):
    """HTMLParser locates real start tags; only their URL value spans are replaced."""
    lines = [0]
    for match in re.finditer("\n", text):
        lines.append(match.end())
    edits = []
    convert = lambda value: relative_url(value, page, output)

    class Tokens(HTMLParser):
        def handle_starttag(self, tag, attrs):
            raw = self.get_starttag_text()
            offset = lines[self.getpos()[0] - 1] + self.getpos()[1]
            values = dict(attrs)
            first = re.match(r"<\s*[^\s/>]+", raw).end()
            for match in ATTRIBUTE.finditer(raw, first):
                name = match[1].lower()
                group = next((n for n in (2, 3, 4) if match[n] is not None), None)
                if group is None:
                    continue
                original = unescape(match[group])
                updated = original
                if tag == "base" and name == "href":
                    # mdBook emits a root base on its 404. Keep resource resolution
                    # in that book when opened directly from a portable bundle.
                    updated = quote(page.name)
                elif name in URL_ATTRIBUTES:
                    updated = convert(original)
                elif name == "srcset":
                    updated = srcset_urls(original, convert)
                elif name == "style":
                    updated = css_urls(original, page, output)
                elif tag == "meta" and name == "content":
                    if values.get("http-equiv", "").lower() == "refresh":
                        refresh = re.match(
                            r'(?is)(.*?;\s*url\s*=\s*)([\'"]?)(.*?)(\2)\s*$', original
                        )
                        if refresh:
                            updated = (
                                refresh[1]
                                + refresh[2]
                                + convert(refresh[3])
                                + refresh[4]
                            )
                    elif (
                        values.get("property", values.get("name", "")).lower()
                        in META_URLS
                    ):
                        updated = convert(original)
                if updated != original:
                    replacement = escape(updated, quote=True)
                    # Unquoted attributes must remain one token even if a URL has spaces.
                    edits.append(
                        (
                            offset + match.start(group),
                            offset + match.end(group),
                            replacement,
                        )
                    )

        handle_startendtag = handle_starttag

    parser = Tokens(convert_charrefs=False)
    parser.feed(text)
    parser.close()
    return apply_edits(text, edits)


def css_urls(text, page, output):
    """Scan CSS tokens so url(...) inside comments or string content is untouched."""
    edits = []
    cursor = 0
    while cursor < len(text):
        if text.startswith("/*", cursor):
            end = text.find("*/", cursor + 2)
            cursor = len(text) if end < 0 else end + 2
            continue
        if text[cursor] in "'\"":
            delimiter = text[cursor]
            cursor += 1
            while cursor < len(text):
                if text[cursor] == "\\":
                    cursor += 2
                elif text[cursor] == delimiter:
                    cursor += 1
                    break
                else:
                    cursor += 1
            continue
        token = re.match(r"(?i)url\(\s*", text[cursor:])
        if token and (
            cursor == 0 or not (text[cursor - 1].isalnum() or text[cursor - 1] in "_-")
        ):
            start = cursor + token.end()
            delimiter = (
                text[start] if start < len(text) and text[start] in "'\"" else ")"
            )
            start += delimiter != ")"
            end = text.find(delimiter, start)
            if end >= 0:
                original = text[start:end]
                trimmed = original.rstrip()
                if "\\" not in trimmed:
                    edits.append(
                        (
                            start,
                            start + len(trimmed),
                            relative_url(trimmed, page, output),
                        )
                    )
                cursor = end + 1
                continue
        cursor += 1
    return apply_edits(text, edits)


def prepare_portable(output):
    """Leave Pages output alone; call only for an explicitly portable build."""
    for page in output.rglob("*.html"):
        original = page.read_bytes().decode("utf-8")
        updated = html_urls(original, page, output)
        if updated != original:
            page.write_bytes(updated.encode("utf-8"))
    for sheet in output.rglob("*.css"):
        original = sheet.read_bytes().decode("utf-8")
        updated = css_urls(original, sheet, output)
        if updated != original:
            sheet.write_bytes(updated.encode("utf-8"))
    # Portable copies are browsing artifacts, not a production crawler destination.
    (output / "sitemap.xml").unlink(missing_ok=True)
    (output / "robots.txt").unlink(missing_ok=True)
