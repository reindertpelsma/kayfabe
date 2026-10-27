"""Phrases the frontpage and the README must never contain.

Each entry is (regex, why). The list comes from the project's do-not-claim review of 2026-10-27;
a phrase is banned because the evidence does not support it today, not because it is rude.
`find(text, readme=False)` returns the matches; the frontpage gets the stricter list.
"""

import re

BOTH = [
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
]

FRONTPAGE_ONLY = [
    (r"\bany\s+(nvidia\s+)?gpu\b", "'any GPU' is a goal, not a claim"),
    (r"\bsafe\s+(for|to run)\s+untrusted", "not audited"),
    (r"\bparavirtual", "kayfabe emulates a GPU, it is not a paravirtual interface"),
]


def find(text, readme=False):
    rules = BOTH if readme else BOTH + FRONTPAGE_ONLY
    return [(m.group(0), why) for rx, why in rules for m in re.finditer(rx, text, re.I)]
