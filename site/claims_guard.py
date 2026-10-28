"""Phrases the frontpage and the README must never contain.

Each entry is (regex, why). The list comes from the project's do-not-claim review of 2026-10-27;
a phrase is banned because the evidence does not support it today, not because it is rude.
`find(text, readme=False)` returns the matches; the frontpage gets the stricter list.
"""

import re

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

# Plain words only in the blocks a first-time reader sees (hero, status band, pitch, isolation, audience cards).
JARGON = [
    r"/dev/\w+", r"\bphysical\b", r"\bunprivileged\b", r"\bCAP_\w+", r"\bioctl\b", r"\bpage[- ]tables?\b",
    r"\bIOMMU\b", r"\bPTE\b", r"\bBAR[0-9]\b", r"\bdoorbell", r"\bcapabilit(y|ies)\b",
]


def jargon(text):
    return [m.group(0) for rx in JARGON for m in re.finditer(rx, text, re.I)]

FRONTPAGE_ONLY = [
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
