"""Phrases the frontpage and the README must never contain.

Each entry is (regex, why). The list comes from the project's do-not-claim review of 2026-10-27;
a phrase is banned because the evidence does not support it today, not because it is rude.
`find(text, readme=False)` returns the matches; the frontpage gets the stricter list.
"""

import re
from html.parser import HTMLParser

BOTH = [
    (r"production path|experimental build|research build", "stale labels: lazy observe and the pointer lifecycle are on master; re-measurement pending"),
    (r"\b(89|102|212|233|296|299|338|362)\b[^.\n]{0,12}fps|\b330\b[^.\n]{0,12}fps|\b342\b[^.\n]{0,12}fps", "no Windows Heaven fps figure until the re-measurement (PF2) lands"),
    (r"near[- ]native", "speed claims need the table, with the production path next to native"),
    (r"\bsecure(ly)?\b", "the security model is designed, not audited"),
    (r"clean[- ]room", "the provenance policy makes no clean-room claim"),
    (r"reverse[- ]engineer", "nothing in public copy may mention reverse engineering"),
    (r"anti-?cheat[- ]+(compatible|friendly|proof|ready)", "anti-cheat compatibility is undocumented"),
    (r"multi-?tenant[- ]+(ready|safe|proof)", "several tenants on one card are not measured"),
    (r"(works?|runs?|supports?)\s+(with|on)\s+any\s+(nvidia\s+)?gpu", "Turing and newer only"),
    (r"\b(revolutionary|seamless|blazing|game-changing|cutting-edge)\b", "hype words"),
    (r"replaces\s+nvkvm-pv", "a goal, not today's fact"),
    (r"\bcertified\b(?!\s+by)", "no vendor certifies this setup"),
    (r"never\s+(hands|gives|exposes|shows)[^.]{0,40}physical", "the guest does use real GPU memory, through virtual addresses; say that"),
    (r"(guaranteed|impossible)\s+(isolation|for the guest)", "isolation is designed, not audited"),
    (r"\binsecure\s+without", "confinement is an extra layer, not a precondition"),
    (r"(fully|completely)\s+(isolated|sandboxed)", "not audited"),
]

# Keep the hero free of the two low-level terms named by the owner.
JARGON = [r"/dev/\w+", r"\bphysical\b"]


def jargon(text):
    return [m.group(0) for rx in JARGON for m in re.finditer(rx, text, re.I)]

FRONTPAGE_ONLY = [
    (r"\b\d+(?:\.\d+)?\s*(?:fps|frames? per second)\b",
     "no kayfabe fps figures in site copy"),
    (r"\bof native\b|\d+ to \d+%", "no percent-of-native headline: the re-measurement is pending"),
    (r"[\u2014\u2013]", "no em or en dashes in site copy"),
    (r"\bnot just\b[^.]{0,60}\bbut\b", "'not just X but Y' is a tell"),
    (r"\bwhether you('re| are)\b", "'whether you are A, B or C' is a tell"),
    (r"worth noting|\bimportantly\b|\bin essence\b", "filler openers"),
    (r"\b(leverage|unlock|empower|robust|powerful)\b", "empty words"),
    (r"\bany\s+(nvidia\s+)?gpu\b", "'any GPU' is a goal, not a claim"),
    (r"\bsafe\s+(for|to run)\s+untrusted", "not audited"),
    (r"\bparavirtual", "kayfabe emulates a GPU, it is not a paravirtual interface"),
]


def find(text, readme=False):
    rules = BOTH if readme else BOTH + FRONTPAGE_ONLY
    return [(m.group(0), why) for rx, why in rules for m in re.finditer(rx, text, re.I)]


# These are prose measurements, not a dictionary or a reading-level guarantee. Keep the
# syllable estimate explicit: count vowel groups, subtract a silent trailing e (except le),
# and use at least one syllable. Product names, abbreviations and numbers are approximate.
# Decimal versions count as one word. Fragments in a list or table cell count as sentences;
# headings, navigation, code blocks and source comments do not pad the prose average.
WORD = re.compile(r"\d+(?:\.\d+)+|[^\W_]+(?:['’\-][^\W_]+)*", re.UNICODE)
SENTENCE_END = re.compile(r"[.!?]+[\"'’”)]*(?:\s+|$)")
VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "wbr"}
SKIP = {"head", "script", "style", "template", "noscript", "svg", "nav", "pre"}
BLOCKS = {"p", "li", "figcaption", "td", "th", "dd", "dt"}


class Element:
    def __init__(self, tag="root", attrs=()):
        self.tag = tag
        self.attrs = dict(attrs)
        self.children = []


class CopyTree(HTMLParser):
    def __init__(self, html):
        super().__init__(convert_charrefs=True)
        self.root = Element()
        self.stack = [self.root]
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        node = Element(tag, attrs)
        self.stack[-1].children.append(node)
        if tag not in VOID:
            self.stack.append(node)

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in VOID:
            self.handle_endtag(tag)

    def handle_endtag(self, tag):
        for index in range(len(self.stack) - 1, 0, -1):
            if self.stack[index].tag == tag:
                del self.stack[index:]
                break

    def handle_data(self, text):
        self.stack[-1].children.append(text)


def omitted(node):
    style = node.attrs.get("style", "").replace(" ", "").lower()
    return (
        node.tag in SKIP
        or "hidden" in node.attrs
        or "display:none" in style
        or "visibility:hidden" in style
    )


def text_of(node, omit_lists=False):
    if isinstance(node, str):
        return node
    if omitted(node) or (omit_lists and node.tag in {"ul", "ol"}):
        return ""
    return " ".join(text_of(child, omit_lists) for child in node.children)


def first_tag(node, tag):
    if isinstance(node, str):
        return None
    if node.tag == tag:
        return node
    return next((found for child in node.children if (found := first_tag(child, tag))), None)


def reading_stats(blocks):
    lengths = []
    syllables = 0
    for block in blocks:
        for sentence in SENTENCE_END.split(block.strip()):
            words = WORD.findall(sentence)
            if not words:
                continue
            lengths.append(len(words))
            for word in words:
                lower = word.lower()
                estimate = len(re.findall(r"[aeiouy]+", lower))
                if lower.endswith("e") and not lower.endswith("le"):
                    estimate -= 1
                syllables += max(1, estimate)
    count = sum(lengths)
    average = count / len(lengths) if lengths else 0
    ease = 206.835 - 1.015 * average - 84.6 * syllables / count if count else None
    return {
        "words": count,
        "sentences": len(lengths),
        "average_sentence_words": round(average, 3),
        "longest_sentence_words": max(lengths, default=0),
        "flesch_reading_ease": round(ease, 2) if ease is not None else None,
    }


def page_review(html):
    """Hard: honesty, the hero's two jargon checks, average<=20, each sentence<=35.

    Flesch<50, paragraph>3 sentences, lists>1 level and sections>5 prose blocks are
    advisory. There is deliberately no word-count limit. All checks use rendered main
    content, with hidden markup and comments omitted; the manual is excluded by build.py.
    """
    tree = CopyTree(html).root
    scope = first_tag(tree, "main") or first_tag(tree, "body") or tree
    sections = []
    current = {"heading": "Introduction", "blocks": [], "paragraph_sentences": [], "list_depth": 0}
    sections.append(current)
    hero_text = []

    def walk(node, list_depth=0, in_block=False):
        nonlocal current
        if isinstance(node, str) or omitted(node):
            return
        if node.tag == "section" and "hero" in node.attrs.get("class", "").split():
            hero_text.append(text_of(node))
        if node.tag in {"h1", "h2"}:
            if current["blocks"]:
                current = {
                    "heading": text_of(node).strip(),
                    "blocks": [],
                    "paragraph_sentences": [],
                    "list_depth": 0,
                }
                sections.append(current)
            else:
                current["heading"] = text_of(node).strip()
            return
        if node.tag in {"ul", "ol"}:
            list_depth += 1
            current["list_depth"] = max(current["list_depth"], list_depth)
        block = node.tag in BLOCKS and not in_block
        if block:
            value = " ".join(text_of(node, omit_lists=node.tag == "li").split())
            if value:
                current["blocks"].append(value)
                if node.tag == "p":
                    current["paragraph_sentences"].append(reading_stats([value])["sentences"])
        for child in node.children:
            # A nested list has its own blocks; inline tags belong to their enclosing block.
            nested_list = isinstance(child, Element) and child.tag in {"ul", "ol"}
            walk(child, list_depth, (in_block or block) and not nested_list)

    walk(scope)
    sections = [s for s in sections if s["blocks"]]
    all_blocks = [text for section in sections for text in section["blocks"]]
    result = {"page": reading_stats(all_blocks), "sections": [], "errors": [], "warnings": []}
    for match, why in find(text_of(scope)):
        result["errors"].append(f"claim/style phrase {match!r}: {why}")
    for match in jargon(" ".join(hero_text)):
        result["errors"].append(f"hero jargon: {match}")
    for section in sections:
        stats = reading_stats(section["blocks"])
        row = {
            "heading": section["heading"],
            **stats,
            "blocks": len(section["blocks"]),
            "max_paragraph_sentences": max(section["paragraph_sentences"], default=0),
            "list_depth": section["list_depth"],
        }
        result["sections"].append(row)
        if stats["flesch_reading_ease"] is not None and stats["flesch_reading_ease"] < 50:
            result["warnings"].append(
                f"{row['heading']}: Flesch {stats['flesch_reading_ease']} < 50"
            )
        for metric, limit in [("blocks", 5), ("max_paragraph_sentences", 3), ("list_depth", 1)]:
            if row[metric] > limit:
                result["warnings"].append(f"{row['heading']}: {metric}={row[metric]} > {limit}")
    for name, stats in [("page", result["page"])] + [(s["heading"], s) for s in result["sections"]]:
        if stats["average_sentence_words"] > 20:
            result["errors"].append(
                f"{name}: average sentence > 20 words " f"({stats['average_sentence_words']})"
            )
        if stats["longest_sentence_words"] > 35:
            result["errors"].append(
                f"{name}: sentence > 35 words " f"({stats['longest_sentence_words']})"
            )
    return result


def enforce_page(html, page):
    result = page_review(html)
    if result["errors"]:
        raise ValueError(f"Published copy {page}: " + "; ".join(result["errors"]))
    return result
