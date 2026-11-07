"""FALSIFIER: a colour pair in either theme falls below WCAG AA (4.5:1 text, 3:1 focus ring), the
dark tokens of the media query and of the data-theme override drift apart, a stylesheet loads
anything from the network, or a promised style (focus, reduced motion, print, no horizontal page
scroll helpers) goes missing. The stylesheets are read as text; no browser is needed."""

from pathlib import Path
import re
import unittest

SITE = Path(__file__).resolve().parent
SHEETS = [SITE / "static/style.css", SITE / "status/status.css"]


def declarations(block):
    return dict(re.findall(r"(--[a-z0-9-]+)\s*:\s*([^;]+);", block))


def blocks(css, selector, indent=r"\s*"):
    """Bodies of every rule whose selector is exactly `selector`."""
    out = []
    for match in re.finditer(r"(?m)^" + indent + re.escape(selector) + r"\s*\{([^{}]*)\}", css):
        out.append(match.group(1))
    return out


def luminance(hex_color):
    value = hex_color.strip().lstrip("#")
    if len(value) == 3:
        value = "".join(c * 2 for c in value)
    r, g, b = (int(value[i : i + 2], 16) / 255 for i in (0, 2, 4))
    lin = lambda c: c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4
    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)


def ratio(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def themes():
    light, dark_media, dark_attr = {}, {}, {}
    for sheet in SHEETS:
        css = sheet.read_text()
        for body in blocks(css, ":root", indent=""):
            light.update(declarations(body))
        for body in blocks(css, ':root:not([data-theme="light"])'):
            dark_media.update(declarations(body))
        for body in blocks(css, ':root[data-theme="dark"]'):
            dark_attr.update(declarations(body))
    return light, dark_media, dark_attr


TEXT_PAIRS = [
    ("--fg", "--bg"),
    ("--fg", "--surface"),
    ("--fg", "--surface-2"),
    ("--muted", "--bg"),
    ("--muted", "--surface"),
    ("--link", "--bg"),
    ("--link", "--surface"),
    ("--on-accent", "--accent"),
    ("--fg", "--warn-bg"),
    ("--measured-fg", "--measured-bg"),
    ("--inferred-fg", "--inferred-bg"),
    ("--gap-fg", "--gap-bg"),
    ("--kf-pass-fg", "--kf-pass-bg"),
    ("--kf-fail-fg", "--kf-fail-bg"),
    ("--kf-retry-fg", "--kf-retry-bg"),
    ("--kf-unavailable-fg", "--kf-unavailable-bg"),
    ("--kf-running-fg", "--kf-running-bg"),
    ("--kf-imp-fg", "--kf-imp-bg"),
    ("--kf-fail-fg", "--bg"),
    ("--muted", "--kf-retry-bg"),
]


class Contrast(unittest.TestCase):
    def setUp(self):
        self.light, self.dark_media, self.dark_attr = themes()

    def check(self, tokens, name):
        for fg, bg in TEXT_PAIRS:
            if fg not in tokens or bg not in tokens:
                continue
            value = ratio(tokens[fg], tokens[bg])
            self.assertGreaterEqual(value, 4.5, f"{name}: {fg} on {bg} is {value:.2f}:1")

    def test_light_pairs_pass_aa(self):
        self.assertGreater(len(self.light), 30)
        self.check(self.light, "light")

    def test_dark_pairs_pass_aa(self):
        self.check({**self.light, **self.dark_attr}, "dark")

    def test_focus_ring_is_visible_in_both_themes(self):
        for name, tokens in (("light", self.light), ("dark", {**self.light, **self.dark_attr})):
            for surface in ("--bg", "--surface"):
                self.assertGreaterEqual(ratio(tokens["--accent"], tokens[surface]), 3.0, f"{name} {surface}")

    def test_media_query_and_override_hold_the_same_dark_tokens(self):
        self.assertEqual(self.dark_media, self.dark_attr)
        for token in ("--bg", "--fg", "--link", "--kf-pass-bg"):
            self.assertIn(token, self.dark_attr)

    def test_every_dark_token_has_a_light_value(self):
        self.assertEqual(sorted(set(self.dark_attr) - set(self.light) - {"--shadow"}), [])


class Promises(unittest.TestCase):
    def setUp(self):
        self.css = "".join(path.read_text() for path in SHEETS)
        self.book = (SITE / "book-theme/kayfabe.css").read_text()

    def test_nothing_is_loaded_from_the_network(self):
        for text in (self.css, self.book):
            self.assertIsNone(re.search(r"@import|url\(|https?:", text, re.I))
        self.assertEqual((SITE / "book-theme/fonts/fonts.css").read_text().count("url("), 0)

    def test_system_font_stack_only(self):
        self.assertNotIn("@font-face", self.css + self.book)
        self.assertIn("system-ui", self.css)

    def test_focus_motion_print_and_colour_scheme_are_handled(self):
        for text in (self.css, self.book):
            self.assertIn(":focus-visible", text)
            self.assertIn("prefers-reduced-motion: reduce", text)
            self.assertIn("@media print", text)
        self.assertIn("prefers-color-scheme: dark", self.css)

    def test_tables_scroll_inside_their_own_container(self):
        self.assertIn("overflow-x: auto", self.css)
        self.assertIn(".table-scroll", self.css)
        self.assertIn(".kf-scroll", self.css)

    def test_status_chips_carry_a_glyph_as_well_as_colour(self):
        for name in ("pass", "fail", "retry", "unattributed", "running", "refused"):
            self.assertRegex(self.css, r"\.kf-v-" + name + r"[^{]*::before\s*\{[^}]*content:", name)
        self.assertRegex(self.css, r"\.kf-imp::before\s*\{[^}]*content:")

    def test_header_menu_is_a_disclosure_that_is_open_without_script(self):
        for path in (SITE / "templates/base.html", SITE / "status_pages.py"):
            self.assertIn('<details class="menu" open>', path.read_text(), path.name)


if __name__ == "__main__":
    unittest.main()
