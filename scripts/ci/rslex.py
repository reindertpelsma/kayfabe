# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A small Rust tokenizer for the perimeter gates (docs/design/V3_SEC_PERIMETER.md §1.4).

Why a tokenizer and not a grep: the old gates decided "code or comment" by the first
character of a line, and `*` is both a comment continuation and the dereference operator
(audit S1-01). A token stream answers the question exactly.

What it handles: nested block comments; doc comments (kept, with their text, for L10);
raw strings with any number of hashes; byte, C and raw byte/C strings; char and byte
literals versus lifetimes and labels; raw identifiers (`r#unsafe` is an identifier named
`unsafe`, not the keyword). It FAILS CLOSED: an unterminated string, char or comment raises
`LexError` rather than returning a short token list, because a short list is how a gate
reports "clean" for a file it never finished reading.

What it does not do: parse Rust. `items()` below recognises item headers, bodies and
attributes well enough for the gates, and every gate that uses it has known-positive
fixtures in `scripts/ci/test_unsafe_gates.py`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

KEYWORD_UNSAFE = "un" + "safe"


class LexError(ValueError):
    """The file could not be tokenized; the gate must fail, never skip it."""


@dataclass(frozen=True)
class Tok:
    kind: str  # ident | lifetime | punct | str | char | num | comment | doc
    text: str  # identifiers: the name (raw prefix stripped); literals: the source text
    line: int
    col: int
    end_line: int
    raw: bool = False  # r#ident
    inner: bool = False  # doc: //! or /*!

    @property
    def is_code(self) -> bool:
        return self.kind not in ("comment", "doc")

    def is_ident(self, name: str | None = None) -> bool:
        return self.kind == "ident" and not self.raw and (name is None or self.text == name)

    def is_punct(self, ch: str) -> bool:
        return self.kind == "punct" and self.text == ch


def _ident_start(c: str) -> bool:
    return c == "_" or c.isalpha() or (ord(c) > 127 and c.isidentifier())


def _ident_cont(c: str) -> bool:
    return c == "_" or c.isalnum() or (ord(c) > 127 and ("a" + c).isidentifier())


def tokenize(src: str, name: str = "<src>") -> list[Tok]:
    toks: list[Tok] = []
    i, n = 0, len(src)
    line, col0 = 1, 0  # col0: index of the first char of the current line

    def err(msg: str) -> LexError:
        return LexError(f"{name}:{line}: {msg}")

    # A shebang line is not Rust (`#![` is an inner attribute, not a shebang).
    if src.startswith("#!") and not src[2:].lstrip().startswith("["):
        while i < n and src[i] != "\n":
            i += 1

    def advance_to(j: int) -> None:
        nonlocal i, line, col0
        seg = src[i:j]
        nl = seg.count("\n")
        if nl:
            line += nl
            col0 = i + seg.rfind("\n") + 1
        i = j

    def emit(kind: str, text: str, start: int, sline: int, scol: int, **kw) -> None:
        toks.append(Tok(kind, text, sline, scol, line, **kw))
        del start

    def quoted(j: int, quote: str) -> int:
        """Index just past the closing quote of an escaped literal whose body starts at j."""
        while j < n:
            c = src[j]
            if c == "\\":
                j += 2
                continue
            if c == quote:
                return j + 1
            j += 1
        raise err(f"unterminated {quote}-literal")

    def raw_string(j: int) -> int | None:
        """j at the char after the `r`; returns the end index, or None if not a raw string."""
        k = j
        while k < n and src[k] == "#":
            k += 1
        if k < n and src[k] == '"':
            hashes = k - j
            close = '"' + "#" * hashes
            end = src.find(close, k + 1)
            if end < 0:
                raise err("unterminated raw string")
            return end + len(close)
        return None

    while i < n:
        c = src[i]
        sline, scol = line, i - col0 + 1
        if c == "\n" or c.isspace():
            advance_to(i + 1)
            continue
        # comments
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            text = src[i:j]
            is_doc = (text.startswith("///") and not text.startswith("////")) or text.startswith("//!")
            advance_to(j)
            emit("doc" if is_doc else "comment", text, i, sline, scol, inner=text.startswith("//!"))
            continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while depth:
                if j >= n:
                    raise err("unterminated block comment")
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            text = src[i:j]
            is_doc = ((text.startswith("/**") and not text.startswith("/***") and text != "/**/")
                      or text.startswith("/*!"))
            advance_to(j)
            emit("doc" if is_doc else "comment", text, i, sline, scol, inner=text.startswith("/*!"))
            continue
        # prefixed literals and raw identifiers: r" r#" b" br" c" cr" b' r#ident
        if c in "rbc":
            j = i + 1
            if c in "bc" and j < n and src[j] == "r":
                end = raw_string(j + 1)
                if end is not None:
                    text = src[i:end]
                    advance_to(end)
                    emit("str", text, i, sline, scol)
                    continue
            if c == "r":
                end = raw_string(j)
                if end is not None:
                    text = src[i:end]
                    advance_to(end)
                    emit("str", text, i, sline, scol)
                    continue
                if src.startswith("#", j) and j + 1 < n and _ident_start(src[j + 1]):
                    k = j + 1
                    while k < n and _ident_cont(src[k]):
                        k += 1
                    text = src[j + 1:k]
                    advance_to(k)
                    emit("ident", text, i, sline, scol, raw=True)
                    continue
            if c in "bc" and j < n and src[j] == '"':
                end = quoted(j + 1, '"')
                text = src[i:end]
                advance_to(end)
                emit("str", text, i, sline, scol)
                continue
            if c == "b" and j < n and src[j] == "'":
                end = quoted(j + 1, "'")
                text = src[i:end]
                advance_to(end)
                emit("char", text, i, sline, scol)
                continue
        if _ident_start(c):
            j = i + 1
            while j < n and _ident_cont(src[j]):
                j += 1
            text = src[i:j]
            advance_to(j)
            emit("ident", text, i, sline, scol)
            continue
        if c.isdigit():
            j = i + 1
            while j < n and (_ident_cont(src[j]) or (src[j] == "." and j + 1 < n and src[j + 1].isdigit()
                                                     and src[j - 1] != ".")):
                j += 1
            text = src[i:j]
            advance_to(j)
            emit("num", text, i, sline, scol)
            continue
        if c == '"':
            end = quoted(i + 1, '"')
            text = src[i:end]
            advance_to(end)
            emit("str", text, i, sline, scol)
            continue
        if c == "'":
            # char literal or lifetime/label
            if i + 1 < n and src[i + 1] == "\\":
                end = quoted(i + 1, "'")
                text = src[i:end]
                advance_to(end)
                emit("char", text, i, sline, scol)
                continue
            if i + 2 < n and src[i + 2] == "'" and src[i + 1] != "\n":
                text = src[i:i + 3]
                advance_to(i + 3)
                emit("char", text, i, sline, scol)
                continue
            j = i + 1
            if j < n and src.startswith("r#", j):
                j += 2
            if j < n and _ident_start(src[j]):
                while j < n and _ident_cont(src[j]):
                    j += 1
                text = src[i:j]
                advance_to(j)
                emit("lifetime", text, i, sline, scol)
                continue
            raise err("unterminated char literal")
        advance_to(i + 1)
        emit("punct", c, i, sline, scol)
    return toks


def tokenize_file(path: Path, rel: str | None = None) -> list[Tok]:
    try:
        src = path.read_text(encoding="utf-8")
    except UnicodeDecodeError as e:
        raise LexError(f"{rel or path}: not UTF-8 ({e})") from e
    return tokenize(src, rel or str(path))


# ---------------------------------------------------------------------------------------
# Structure over the code-token stream
# ---------------------------------------------------------------------------------------

OPEN = {"(": ")", "[": "]", "{": "}"}
CLOSE = {v: k for k, v in OPEN.items()}

# Tokens after which an item may start (an attribute's `]`, a block or statement end, or
# the start of the file), and qualifiers that may stand between that point and the
# item keyword.
QUALIFIERS = {"pub", "crate", "const", "async", "default", "safe", KEYWORD_UNSAFE, "extern", "auto"}


@dataclass
class Item:
    kind: str  # fn | impl | trait | mod | extern_block | macro_rules | struct | enum | union | ...
    name: str
    start: int  # code-token index of the first token (attributes included)
    kw: int  # code-token index of the keyword
    body_open: int | None  # index of `{` (or macro delimiter) if any
    end: int  # index of the last token of the item
    attrs: list[tuple[int, int]] = field(default_factory=list)  # (open `#`, close `]`)
    qualifiers: set[str] = field(default_factory=set)
    parent: "Item | None" = None


class Structure:
    """Code tokens, bracket matching, attributes and item headers for one file."""

    def __init__(self, toks: list[Tok], name: str = "<src>"):
        self.all = toks
        self.code = [t for t in toks if t.is_code]
        self.name = name
        self.match: dict[int, int] = {}
        stack: list[int] = []
        for k, t in enumerate(self.code):
            if t.kind != "punct":
                continue
            if t.text in OPEN:
                stack.append(k)
            elif t.text in CLOSE:
                if not stack or self.code[stack[-1]].text != CLOSE[t.text]:
                    raise LexError(f"{name}:{t.line}: unbalanced '{t.text}'")
                o = stack.pop()
                self.match[o] = k
                self.match[k] = o
        if stack:
            raise LexError(f"{name}:{self.code[stack[-1]].line}: unclosed '{self.code[stack[-1]].text}'")
        self.attrs = self._attrs()
        self.items = self._items()

    # -- attributes -------------------------------------------------------------------
    def _attrs(self) -> list[tuple[int, int, bool]]:
        out = []
        c = self.code
        for k, t in enumerate(c):
            if not t.is_punct("#"):
                continue
            j = k + 1
            inner = j < len(c) and c[j].is_punct("!")
            if inner:
                j += 1
            if j < len(c) and c[j].is_punct("["):
                out.append((k, self.match[j], inner))
        return out

    def attr_tokens(self, a: tuple[int, int]) -> list[Tok]:
        o, e = a[0], a[1]
        return self.code[o:e + 1]

    def in_attr(self, k: int) -> bool:
        return any(o <= k <= e for o, e, _ in self.attrs)

    # -- items ------------------------------------------------------------------------
    def _next_body(self, k: int) -> tuple[int | None, int]:
        """From k, the first `{` at bracket depth 0 (returns (open, close)) or `;` (None, idx)."""
        c = self.code
        j = k
        while j < len(c):
            t = c[j]
            if t.kind == "punct":
                if t.text == "{":
                    return j, self.match[j]
                if t.text == ";":
                    return None, j
                if t.text in ("(", "["):
                    j = self.match[j] + 1
                    continue
                if t.text in (")", "]", "}"):
                    # the enclosing group ended without a body: a header-only fragment
                    return None, j - 1
            j += 1
        return None, len(c) - 1

    def _items(self) -> list[Item]:
        items = [it for k in range(len(self.code)) if (it := self._item_at(k)) is not None]
        # parent links: innermost enclosing item whose body contains this item's keyword
        bodies = sorted((it for it in items if it.body_open is not None), key=lambda it: it.body_open)
        for it in items:
            best = None
            for b in bodies:
                if b is not it and b.body_open < it.kw <= b.end:
                    if best is None or b.body_open > best.body_open:
                        best = b
            it.parent = best
        return items

    def _outer_attrs_before(self, j: int) -> list[tuple[int, int]]:
        """The outer attributes ending immediately before code index j, in source order."""
        c = self.code
        out = []
        while j > 0 and c[j - 1].is_punct("]"):
            o = self.match[j - 1]
            if o > 0 and c[o - 1].is_punct("#"):
                out.append((o - 1, j - 1))
                j = o - 1
                continue
            break
        return list(reversed(out))

    def _qualifiers_before(self, kw: int) -> tuple[set[str], int]:
        c = self.code
        quals: set[str] = set()
        j = kw - 1
        while j >= 0:
            t = c[j]
            if t.kind == "ident" and not t.raw and t.text in QUALIFIERS:
                quals.add(t.text)
                j -= 1
                continue
            if t.kind == "str" and j > 0 and c[j - 1].is_ident("extern"):
                j -= 1
                continue
            if t.is_punct(")") and self.match[j] > 0 and c[self.match[j] - 1].is_ident("pub"):
                j = self.match[j] - 1
                continue
            break
        return quals, j + 1

    def _item_at(self, k: int) -> Item | None:
        c = self.code
        t = c[k]
        if t.kind != "ident" or t.raw:
            return None
        nxt = c[k + 1] if k + 1 < len(c) else None
        prev_ok = self._item_position(k)
        _, qstart0 = self._qualifiers_before(k)
        attrs = self._outer_attrs_before(qstart0)
        astart = attrs[0][0] if attrs else None
        if t.text == "macro_rules" and nxt is not None and nxt.is_punct("!") and prev_ok:
            name = c[k + 2].text if k + 2 < len(c) else ""
            if k + 3 < len(c) and c[k + 3].kind == "punct" and c[k + 3].text in OPEN:
                o = k + 3
                e = self.match[o]
                return Item("macro_rules", name, astart if astart is not None else k, k, o, e, list(attrs))
            return None
        kinds = {"fn", "impl", "trait", "mod", "struct", "enum", "union", "type", "static", "const", "use"}
        if t.text == "extern" and prev_ok:
            j = k + 1
            if j < len(c) and c[j].kind == "str":
                j += 1
            if j < len(c) and c[j].is_punct("{"):
                quals, _ = self._qualifiers_before(k)
                return Item("extern_block", "", astart if astart is not None else k, k, j, self.match[j],
                            list(attrs), quals)
            return None
        if t.text not in kinds or not prev_ok:
            return None
        if t.text == "fn":
            if nxt is None or nxt.kind != "ident":
                return None  # a fn-pointer type such as `unsafe fn(u8)`
        if t.text == "const":
            # `const fn`, `const unsafe fn` are qualifiers; `const NAME` / `const _` are items
            if nxt is None or nxt.kind != "ident" or nxt.text in QUALIFIERS or nxt.text == "fn":
                if not (nxt is not None and nxt.is_ident("_")):
                    return None
        if t.text == "union" and (nxt is None or nxt.kind != "ident"):
            return None
        if t.text == "auto":
            return None
        quals, qstart = self._qualifiers_before(k)
        start = astart if astart is not None else qstart
        o, e = self._next_body(k + 1)
        if t.text in ("type", "static", "const", "use"):
            o, e = None, self._stmt_end(k + 1)
        name = ""
        if t.text in ("fn", "mod", "struct", "enum", "union", "trait", "type", "static", "const"):
            j = k + 1
            if t.text == "static" and j < len(c) and c[j].is_ident("mut"):
                j += 1
            name = c[j].text if j < len(c) else ""
        elif t.text == "impl":
            name = self._impl_name(k, o if o is not None else e)
        return Item(t.text, name, start, k, o, e, list(attrs), quals)

    def _stmt_end(self, k: int) -> int:
        c = self.code
        j = k
        while j < len(c):
            t = c[j]
            if t.kind == "punct":
                if t.text == ";":
                    return j
                if t.text in OPEN:
                    j = self.match[j] + 1
                    continue
                if t.text in CLOSE:
                    return j - 1
            j += 1
        return len(c) - 1

    def _impl_name(self, kw: int, stop: int) -> str:
        """`Type` or `Type as Trait` for an impl header (generics dropped)."""
        c = self.code
        toks = [x for x in c[kw + 1:stop] if x.kind in ("ident", "punct", "lifetime")]
        text, depth, out = [], 0, []
        for x in toks:
            if x.is_punct("<"):
                depth += 1
                continue
            if x.is_punct(">"):
                depth = max(0, depth - 1)
                continue
            if depth:
                continue
            if x.is_ident("where"):
                break
            out.append(x.text)
        words = " ".join(out).replace(" :: ", "::").split()
        if "for" in words:
            f = words.index("for")
            trait = "".join(words[:f])
            ty = "".join(words[f + 1:])
            return f"<{ty} as {trait}>"
        del text
        return "".join(words)

    def _item_position(self, k: int) -> bool:
        """Can an item keyword at k start an item (versus `impl Trait` in a type, etc.)?"""
        c = self.code
        _, j = self._qualifiers_before(k)
        if j == 0:
            return True
        p = c[j - 1]
        if p.kind == "punct" and p.text in (";", "{", "}"):
            return True
        if p.is_punct("]"):
            o = self.match[j - 1]
            return o > 0 and (c[o - 1].is_punct("#") or (c[o - 1].is_punct("!") and o > 1 and c[o - 2].is_punct("#")))
        return False

    # -- queries ----------------------------------------------------------------------
    def code_index_of(self) -> dict[int, int]:
        """Map id(Tok) of code tokens to their code index (for mapping comments to context)."""
        return {id(t): k for k, t in enumerate(self.code)}

    def fn_items(self) -> list[Item]:
        return [it for it in self.items if it.kind == "fn"]

    def enclosing(self, k: int, kinds: tuple[str, ...]) -> Item | None:
        best = None
        for it in self.items:
            if it.kind in kinds and it.body_open is not None and it.body_open < k < it.end:
                if best is None or it.body_open > best.body_open:
                    best = it
        return best

    def attr_is(self, a: tuple[int, int], *words: str) -> bool:
        """Does the attribute's token text (without `#[`/`]`) equal `words` exactly?"""
        toks = self.code[a[0]:a[1] + 1]
        inner = [x.text for x in toks]
        # strip `#`, optional `!`, `[` ... `]`
        body = inner[2:-1] if inner[1] == "[" else inner[3:-1]
        return body == list(words)

    def is_cfg_test(self, it: Item) -> bool:
        return any(self.attr_is(a, "cfg", "(", "test", ")") for a in it.attrs)

    def test_ranges(self) -> list[tuple[int, int]]:
        """Code-token ranges of items under exactly `#[cfg(test)]` (outermost only)."""
        rs = sorted((it.start, it.end) for it in self.items if self.is_cfg_test(it))
        out: list[tuple[int, int]] = []
        for s, e in rs:
            if out and s <= out[-1][1]:
                continue
            out.append((s, e))
        return out


def in_ranges(k: int, ranges: list[tuple[int, int]]) -> bool:
    return any(s <= k <= e for s, e in ranges)


# ---------------------------------------------------------------------------------------
# What each `unsafe` token is (the size ratchet's kinds, V3_SEC_PERIMETER.md §2)
# ---------------------------------------------------------------------------------------

KINDS = ("blocks", "unsafe_fn", "unsafe_method", "unsafe_impl", "unsafe_trait", "extern_blocks",
         "extern_items", "unsafe_attrs", "asm")
ASM_MACROS = ("asm", "global_asm", "naked_asm")


@dataclass(frozen=True)
class Site:
    kind: str  # one of KINDS, or "fn_pointer_type" (not counted), or "unknown" (a gate error)
    line: int
    k: int  # code-token index


def _fn_after(c: list[Tok], j: int) -> int | None:
    """Index of `fn` if tokens from j are `[extern ["abi"]] fn`, else None."""
    if j < len(c) and c[j].is_ident("extern"):
        j += 1
        if j < len(c) and c[j].kind == "str":
            j += 1
    return j if j < len(c) and c[j].is_ident("fn") else None


def unsafe_sites(s: Structure) -> list[Site]:
    c = s.code
    out: list[Site] = []
    fn_by_kw = {it.kw: it for it in s.items if it.kind == "fn"}
    for k, t in enumerate(c):
        if t.kind == "ident" and not t.raw and t.text in ASM_MACROS and k + 1 < len(c) and c[k + 1].is_punct("!") \
                and not (k > 0 and c[k - 1].is_ident("macro_rules")):
            out.append(Site("asm", t.line, k))
            continue
        if not t.is_ident(KEYWORD_UNSAFE):
            continue
        nxt = c[k + 1] if k + 1 < len(c) else None
        if s.in_attr(k):
            out.append(Site("unsafe_attrs", t.line, k))
        elif nxt is not None and nxt.is_punct("{"):
            out.append(Site("blocks", t.line, k))
        elif nxt is not None and nxt.is_ident("impl"):
            out.append(Site("unsafe_impl", t.line, k))
        elif nxt is not None and (nxt.is_ident("trait") or (nxt.is_ident("auto") and k + 2 < len(c)
                                                            and c[k + 2].is_ident("trait"))):
            out.append(Site("unsafe_trait", t.line, k))
        elif (j := _fn_after(c, k + 1)) is not None:
            if j + 1 < len(c) and c[j + 1].kind == "ident":
                it = fn_by_kw.get(j)
                parent = it.parent.kind if it is not None and it.parent is not None else None
                if parent == "extern_block":
                    out.append(Site("extern_items", t.line, k))
                    continue
                out.append(Site("unsafe_method" if parent in ("impl", "trait") else "unsafe_fn", t.line, k))
            elif j + 1 < len(c) and (c[j + 1].is_punct("(") or c[j + 1].is_punct("<")):
                out.append(Site("fn_pointer_type", t.line, k))
            elif j + 1 < len(c) and c[j + 1].is_punct("$"):
                out.append(Site("unsafe_fn", t.line, k))  # `unsafe fn $name` in a macro body
            else:
                out.append(Site("unknown", t.line, k))
        elif nxt is not None and nxt.is_ident("extern"):
            j = k + 2
            if j < len(c) and c[j].kind == "str":
                j += 1
            out.append(Site("extern_blocks" if j < len(c) and c[j].is_punct("{") else "unknown", t.line, k))
        else:
            out.append(Site("unknown", t.line, k))
    # the declarations inside extern blocks
    for it in s.items:
        if it.kind in ("fn", "static") and it.parent is not None and it.parent.kind == "extern_block":
            if not any(x.k == it.start for x in out) and not (it.kw > 0 and c[it.kw - 1].is_ident(KEYWORD_UNSAFE)):
                out.append(Site("extern_items", c[it.kw].line, it.kw))
    return out


def macro_bodies(s: Structure) -> list[Item]:
    return [it for it in s.items if it.kind == "macro_rules" and it.body_open is not None]


def macro_invocations(s: Structure, name: str) -> int:
    c = s.code
    return sum(1 for k in range(len(c) - 1)
               if c[k].is_ident(name) and c[k + 1].is_punct("!") and not (k > 0 and c[k - 1].is_ident("macro_rules"))
               and not (k > 0 and c[k - 1].is_punct("$")))
