"""The concurrency block of .github/workflows/ci.yml, evaluated for every kind of trigger.

STATUS: LIVE, 2026-11-17.

FALSIFIER (written before the code): a push to `next` or `master`, a tag push, a scheduled run, a manual dispatch
or a merge-group run can be cancelled by another run, or can share a concurrency group with one; or two pushes
to the same pull-request branch (or two commits of the same pull request) do not share a group and so are not
cancelled by the newer one; or a run that shares a group is not the one marked `cancel-in-progress`.

GitHub is not available here, so this file evaluates the two expressions of the `concurrency:` key with a small
interpreter that knows exactly the subset the workflow uses (`==`, `!=`, `&&`, `||`, parentheses, string
literals, context paths, `format`, `startsWith`) and refuses anything else, so a later edit that adds a
function this file cannot judge fails here instead of being trusted. Semantics follow GitHub's: `&&` and `||`
return an operand, an empty string, zero, null and false are falsy, and string comparison ignores case.
"""

import re
import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"

TOKEN = re.compile(
    r"\s*(?:(?P<str>'(?:[^']|'')*')|(?P<op>==|!=|&&|\|\||[(),])|(?P<name>[A-Za-z_][A-Za-z0-9_.-]*)|(?P<num>[0-9]+))"
)


def tokens(text):
    out, pos = [], 0
    while pos < len(text):
        if not text[pos:].strip():
            break
        m = TOKEN.match(text, pos)
        if not m:
            raise ValueError("cannot tokenize near %r" % text[pos : pos + 20])
        pos = m.end()
        for kind in ("str", "op", "name", "num"):
            if m.group(kind) is not None:
                value = m.group(kind)
                if kind == "str":
                    value = value[1:-1].replace("''", "'")
                out.append((kind, value))
    return out


def truthy(value):
    return value not in (None, False, 0, "")


class Evaluator:
    """or := and ('||' and)* ; and := cmp ('&&' cmp)* ; cmp := atom (('=='|'!=') atom)? ; atom := literal | path | call | '(' or ')'"""

    FUNCTIONS = {"format", "startsWith"}

    def __init__(self, text, context):
        self.toks = tokens(text)
        self.pos = 0
        self.context = context

    def peek(self):
        return self.toks[self.pos] if self.pos < len(self.toks) else (None, None)

    def take(self, kind=None, value=None):
        tok = self.peek()
        if tok[0] is None or (kind and tok[0] != kind) or (value and tok[1] != value):
            raise ValueError("expected %s %s, got %r" % (kind, value, tok))
        self.pos += 1
        return tok

    def parse(self):
        result = self.or_()
        if self.pos != len(self.toks):
            raise ValueError("trailing tokens: %r" % (self.toks[self.pos :],))
        return result

    def or_(self):
        left = self.and_()
        while self.peek() == ("op", "||"):
            self.take()
            right = self.and_()
            left = left if truthy(left) else right
        return left

    def and_(self):
        left = self.cmp()
        while self.peek() == ("op", "&&"):
            self.take()
            right = self.cmp()
            left = right if truthy(left) else left
        return left

    def cmp(self):
        left = self.atom()
        tok = self.peek()
        if tok[0] == "op" and tok[1] in ("==", "!="):
            self.take()
            right = self.atom()
            same = str(left).lower() == str(right).lower()
            return same if tok[1] == "==" else not same
        return left

    def atom(self):
        kind, value = self.peek()
        if kind == "str":
            self.take()
            return value
        if kind == "num":
            self.take()
            return int(value)
        if (kind, value) == ("op", "("):
            self.take()
            inner = self.or_()
            self.take("op", ")")
            return inner
        if kind == "name":
            self.take()
            if self.peek() == ("op", "("):
                return self.call(value)
            if value in ("true", "false", "null"):
                return {"true": True, "false": False, "null": None}[value]
            return self.context_path(value)
        raise ValueError("unexpected token %r" % ((kind, value),))

    def call(self, name):
        if name not in self.FUNCTIONS:
            raise ValueError("function %r is not one this test can judge" % name)
        self.take("op", "(")
        args = []
        while True:
            args.append(self.or_())
            if self.peek() == ("op", ","):
                self.take()
                continue
            self.take("op", ")")
            break
        if name == "format":
            template, rest = args[0], args[1:]
            return re.sub(r"\{(\d+)\}", lambda m: str(rest[int(m.group(1))]), template)
        return str(args[0]).lower().startswith(str(args[1]).lower())

    def context_path(self, path):
        if path in self.context:
            return self.context[path]
        if path.startswith("github.event."):
            return None  # an event payload field that this event does not carry is null, as on GitHub
        raise ValueError("context %r is not provided by this test" % path)


def interpolate(template, context):
    """Evaluate `text ${{ expr }} text` the way GitHub does for a string value."""
    if isinstance(template, bool):
        return template
    parts = re.split(r"\$\{\{(.*?)\}\}", " ".join(str(template).split()), flags=re.S)
    out = []
    for index, part in enumerate(parts):
        out.append(part if index % 2 == 0 else str(Evaluator(part, context).parse()))
    return "".join(out)


def context(event, ref, run_id, pr=None):
    ctx = {"github.event_name": event, "github.ref": ref, "github.run_id": run_id}
    if pr is not None:
        ctx["github.event.pull_request.number"] = pr
    return ctx


def load():
    return yaml.safe_load(WORKFLOW.read_text())["concurrency"]


def outcome(event, ref, run_id, pr=None):
    block = load()
    ctx = context(event, ref, run_id, pr)
    group = interpolate(block["group"], ctx)
    cancel = interpolate(block["cancel-in-progress"], ctx)
    return group, cancel == "True" or cancel is True


class EvaluatorSelfTest(unittest.TestCase):
    def test_and_or_return_operands_and_comparison_ignores_case(self):
        ev = lambda text, ctx=None: Evaluator(text, ctx or {}).parse()
        self.assertEqual(ev("'a' && 'b'"), "b")
        self.assertEqual(ev("'' && 'b' || 'c'"), "c")
        self.assertTrue(ev("'Push' == 'push'"))
        self.assertEqual(ev("format('x-{0}-{1}', 'a', 7)"), "x-a-7")
        self.assertTrue(ev("startsWith('refs/heads/x', 'refs/heads/')"))
        self.assertFalse(ev("startsWith('refs/tags/x', 'refs/heads/')"))

    def test_an_unknown_function_or_context_is_refused_not_trusted(self):
        with self.assertRaises(ValueError):
            Evaluator("contains('a', 'b')", {}).parse()
        with self.assertRaises(ValueError):
            Evaluator("github.actor", {}).parse()
        self.assertIsNone(Evaluator("github.event.pull_request.number", {}).parse())


class CiConcurrency(unittest.TestCase):
    def test_pushes_to_the_protected_branches_are_never_cancelled_and_never_share_a_group(self):
        for ref in ("refs/heads/next", "refs/heads/master"):
            first, cancel_first = outcome("push", ref, 100)
            second, cancel_second = outcome("push", ref, 101)
            self.assertFalse(cancel_first, ref)
            self.assertFalse(cancel_second, ref)
            self.assertNotEqual(first, second, ref)

    def test_tags_schedule_dispatch_and_merge_group_are_never_cancelled_and_never_share_a_group(self):
        for event, ref in (
            ("push", "refs/tags/v1.0"),
            ("schedule", "refs/heads/master"),
            ("workflow_dispatch", "refs/heads/claude/topic-20261117"),
            ("workflow_dispatch", "refs/heads/next"),
            ("merge_group", "refs/heads/gh-readonly-queue/next/pr-1-abc"),
        ):
            first, cancel_first = outcome(event, ref, 200)
            second, cancel_second = outcome(event, ref, 201)
            self.assertFalse(cancel_first, (event, ref))
            self.assertFalse(cancel_second, (event, ref))
            self.assertNotEqual(first, second, (event, ref))

    def test_a_newer_push_to_a_pull_request_branch_shares_the_group_and_cancels(self):
        ref = "refs/heads/claude/topic-20261117"
        old, cancel_old = outcome("push", ref, 300)
        new, cancel_new = outcome("push", ref, 301)
        self.assertEqual(old, new)
        self.assertTrue(cancel_old and cancel_new)

    def test_two_branches_do_not_cancel_each_other(self):
        a, _ = outcome("push", "refs/heads/claude/a-20261117", 400)
        b, _ = outcome("push", "refs/heads/claude/b-20261117", 401)
        self.assertNotEqual(a, b)

    def test_a_newer_commit_of_the_same_pull_request_cancels_and_another_pull_request_does_not(self):
        old, cancel_old = outcome("pull_request", "refs/pull/7/merge", 500, pr=7)
        new, cancel_new = outcome("pull_request", "refs/pull/7/merge", 501, pr=7)
        other, _ = outcome("pull_request", "refs/pull/8/merge", 502, pr=8)
        self.assertEqual(old, new)
        self.assertTrue(cancel_old and cancel_new)
        self.assertNotEqual(old, other)

    def test_the_push_run_and_the_pull_request_run_of_one_branch_do_not_cancel_each_other(self):
        push, _ = outcome("push", "refs/heads/claude/topic-20261117", 600)
        pull, _ = outcome("pull_request", "refs/pull/7/merge", 601, pr=7)
        self.assertNotEqual(push, pull)

    def test_a_branch_named_like_a_run_id_cannot_land_in_the_unshared_namespace(self):
        shared, cancel = outcome("push", "refs/heads/run-700", 700)
        unshared, _ = outcome("schedule", "refs/heads/master", 700)
        self.assertTrue(cancel)
        self.assertNotEqual(shared, unshared)

    def test_every_cancelling_run_is_in_a_group_that_is_not_its_own_run_id(self):
        cases = [
            ("push", "refs/heads/claude/x-1", None),
            ("push", "refs/heads/some-human-branch", None),
            ("pull_request", "refs/pull/3/merge", 3),
        ]
        for event, ref, pr in cases:
            group, cancel = outcome(event, ref, 424242, pr)
            self.assertTrue(cancel, (event, ref))
            self.assertNotIn("424242", group, (event, ref))

    def test_the_gate_still_runs_after_a_cancel(self):
        # A cancelled run's gate must report (red on the superseded commit), never vanish: a gate that is skipped
        # after a cancel would read as a pass for a required check. scripts/ci/test_ci_gate.py runs its verdict.
        gate = yaml.safe_load(WORKFLOW.read_text())["jobs"]["gate"]
        self.assertEqual(gate["if"], "always()")


if __name__ == "__main__":
    sys.exit(unittest.main())
