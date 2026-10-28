"""Unprivileged preparation of a neutral deployment slot, preserving every other byte."""

from html.parser import HTMLParser

SLOT = b"<!-- kf-slot-a -->"
FOOTER = b'<footer id="kf-footer"></footer>'


class Tokens(HTMLParser):
    def __init__(self, text):
        super().__init__(convert_charrefs=False)
        self.tokens, self.positions, self.lines = [], [], text.splitlines(keepends=True)
        self.feed(text)
        self.close()

    def add(self, *token):
        line, column = self.getpos()
        self.positions.append(sum(map(len, self.lines[: line - 1])) + column)
        self.tokens.append(token)

    def handle_starttag(self, tag, attrs):
        self.add("start", tag, tuple(attrs))

    def handle_startendtag(self, tag, attrs):
        self.add("empty", tag, tuple(attrs))

    def handle_endtag(self, tag):
        self.add("end", tag)

    def handle_data(self, value):
        self.add("data", value)

    def handle_comment(self, value):
        self.add("comment", value)

    def handle_entityref(self, value):
        self.add("entity", value)

    def handle_charref(self, value):
        self.add("char", value)

    def handle_decl(self, value):
        self.add("decl", value)

    def unknown_decl(self, value):
        self.add("unknown", value)

    def handle_pi(self, value):
        self.add("pi", value)


def normalized(tokens):
    result = []
    for token in tokens:
        if token[0] == "data" and result and result[-1][0] == "data":
            result[-1] = ("data", result[-1][1] + token[1])
        else:
            result.append(token)
    return result


def splice(raw, tag, addition):
    text = raw.decode("utf-8")
    parsed = Tokens(text)
    targets = [i for i, token in enumerate(parsed.tokens) if token == ("end", tag)]
    if len(targets) != 1:
        raise ValueError("one closing " + tag + " required")
    index = targets[0]
    offset = len(text[: parsed.positions[index]].encode("utf-8"))
    result = raw[:offset] + addition + raw[offset:]
    expected = parsed.tokens[:index] + Tokens(addition.decode()).tokens + parsed.tokens[index:]
    if normalized(Tokens(result.decode("utf-8")).tokens) != normalized(expected):
        raise ValueError("slot preparation changed tokens")
    return result


def prepare_page(raw, candidate):
    parsed = Tokens(raw.decode("utf-8"))
    comments = [t for t in parsed.tokens if t == ("comment", " kf-slot-a ")]
    if len(comments) > 1 or (comments and not candidate):
        raise ValueError("unexpected deployment slot")
    if candidate:
        heads = [t for t in parsed.tokens if t == ("end", "head")]
        if len(heads) != 1:
            raise ValueError("one head end required")
        if not comments:
            raw = splice(raw, "head", SLOT)
    footers = [
        t
        for t in parsed.tokens
        if t[:2] == ("start", "footer") and dict(t[2]).get("id") == "kf-footer"
    ]
    if len(footers) > 1:
        raise ValueError("duplicate footer")
    if not footers:
        raw = splice(raw, "body", FOOTER)
    return raw


def prepare_output(output, candidate=False):
    for path in output.rglob("*.html"):
        raw = path.read_bytes()
        result = prepare_page(raw, candidate)
        if result != raw:
            path.write_bytes(result)


def candidate_context(environment):
    return (
        environment.get("GITHUB_ACTIONS") == "true"
        and environment.get("GITHUB_REPOSITORY") == "reindertpelsma/kayfabe"
        and environment.get("GITHUB_EVENT_NAME")
        in {"push", "release", "schedule", "workflow_dispatch"}
        and (
            environment.get("GITHUB_REF") == "refs/heads/master"
            or environment.get("GITHUB_REF", "").startswith("refs/tags/v")
        )
    )
