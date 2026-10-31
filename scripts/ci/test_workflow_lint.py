import tempfile
import unittest
from pathlib import Path

import workflow_lint as wl

REPO = wl.ROOT
SHA = "11d5960a326750d5838078e36cf38b85af677262"

GOOD = f"""\
name: Good
on: push
permissions:
  contents: read
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@{SHA} # v4.4.0
      - run: echo hi
"""


def tree(**files):
    """A temporary repo root holding .github/workflows/<name> for each keyword (name with _ for .)."""
    d = tempfile.TemporaryDirectory()
    root = Path(d.name)
    (root / ".github" / "workflows").mkdir(parents=True)
    for name, text in files.items():
        (root / ".github" / "workflows" / name.replace("__", ".")).write_text(text)
    return d, root


def lint_one(text, name="x.yml"):
    d, root = tree(**{name.replace(".", "__"): text})
    with d:
        return wl.lint(root)


def real(name):
    return (REPO / ".github" / "workflows" / name).read_text()


def mutate(text, old, new):
    assert old in text, old
    return text.replace(old, new, 1)


class RealRepo(unittest.TestCase):
    def test_current_workflows_pass(self):
        self.assertEqual(wl.lint(REPO), [])

    def test_gpu_vast_files_are_the_unchanged_ones_the_allowlist_describes(self):
        for name in ("gpu-vast.yml", "gpu-vast-sweep.yml"):
            self.assertIn(name, wl.ALLOWLIST)
            self.assertEqual(lint_one(real(name), name), [])

    def test_gpu_job_is_exactly_id_token_write(self):
        self.assertEqual(wl.ALLOWLIST["gpu-vast.yml"]["gpu"], {"id-token": "write"})
        self.assertEqual(wl.ALLOWLIST["gpu-vast-sweep.yml"]["gpu"], {"id-token": "write"})


class Pin(unittest.TestCase):
    def test_sha_pinned_passes(self):
        self.assertEqual(lint_one(GOOD), [])

    def test_tag_fails(self):
        out = lint_one(mutate(GOOD, f"@{SHA}", "@v4"))
        self.assertEqual(len(out), 1)
        self.assertIn("pin:", out[0])

    def test_moving_branch_fails(self):
        out = lint_one(mutate(GOOD, f"actions/checkout@{SHA}", "dtolnay/rust-toolchain@master"))
        self.assertTrue(any("pin:" in x for x in out))

    def test_short_sha_and_uppercase_sha_fail(self):
        self.assertTrue(lint_one(mutate(GOOD, f"@{SHA}", "@11d5960")))
        self.assertTrue(lint_one(mutate(GOOD, f"@{SHA}", f"@{SHA.upper()}")))

    def test_sha_with_extra_suffix_fails(self):
        self.assertTrue(lint_one(mutate(GOOD, f"@{SHA}", f"@{SHA}0")))

    def test_local_action_is_allowed(self):
        self.assertEqual(lint_one(mutate(GOOD, f"actions/checkout@{SHA}", "./.github/actions/x")), [])

    def test_docker_needs_digest(self):
        bad = mutate(GOOD, f"actions/checkout@{SHA}", "docker://alpine:3.20")
        self.assertTrue(lint_one(bad))
        good = mutate(GOOD, f"actions/checkout@{SHA}", "docker://alpine@sha256:" + "a" * 64)
        self.assertEqual(lint_one(good), [])

    def test_uses_text_inside_a_run_block_is_not_an_action(self):
        text = mutate(GOOD, "echo hi", "|\n          echo 'uses: foo/bar@v1'\n          true")
        self.assertEqual(lint_one(text), [])

    def test_quoted_unpinned_still_fails(self):
        self.assertTrue(lint_one(mutate(GOOD, f"actions/checkout@{SHA}", "'actions/checkout@v4'")))

    def test_job_level_reusable_workflow_must_be_pinned(self):
        text = GOOD + "  call:\n    uses: org/repo/.github/workflows/w.yml@main\n"
        self.assertTrue(any("pin:" in x for x in lint_one(text)))

    def test_composite_action_is_checked(self):
        d, root = tree(x__yml=GOOD)
        with d:
            a = root / ".github" / "actions" / "mine"
            a.mkdir(parents=True)
            (a / "action.yml").write_text("runs:\n  using: composite\n  steps:\n    - uses: a/b@v1\n")
            out = wl.lint(root)
            self.assertEqual(len(out), 1)
            self.assertIn("actions/mine/action.yml", out[0])


class Permissions(unittest.TestCase):
    def test_missing_top_level_fails(self):
        out = lint_one(mutate(GOOD, "permissions:\n  contents: read\n", ""))
        self.assertTrue(any("permissions:" in x and "top-level" in x for x in out))

    def test_job_level_only_does_not_count(self):
        text = mutate(GOOD, "permissions:\n  contents: read\njobs:", "jobs:")
        text = mutate(text, "    runs-on: ubuntu-latest\n", "    runs-on: ubuntu-latest\n    permissions:\n      contents: read\n")
        self.assertTrue(lint_one(text))

    def test_empty_top_level_passes(self):
        self.assertEqual(lint_one(mutate(GOOD, "permissions:\n  contents: read\n", "permissions: {}\n")), [])

    def test_write_all_fails(self):
        self.assertTrue(lint_one(mutate(GOOD, "permissions:\n  contents: read\n", "permissions: write-all\n")))
        text = mutate(GOOD, "    runs-on: ubuntu-latest\n", "    runs-on: ubuntu-latest\n    permissions: write-all\n")
        self.assertTrue(lint_one(text))

    def test_unreadable_job_permissions_fail_closed(self):
        text = mutate(GOOD, "    runs-on: ubuntu-latest\n", "    runs-on: ubuntu-latest\n    permissions: ${{ x }}\n")
        self.assertTrue(lint_one(text))


class Allowlist(unittest.TestCase):
    def test_widened_gpu_job_fails(self):
        for name in ("gpu-vast.yml", "gpu-vast-sweep.yml"):
            text = mutate(real(name), "      id-token: write\n", "      id-token: write\n      contents: read\n")
            out = lint_one(text, name)
            self.assertTrue(any("allowlist:" in x and "`gpu`" in x for x in out), out)

    def test_gpu_job_without_permissions_fails(self):
        text = mutate(real("gpu-vast-sweep.yml"), "    permissions:\n      id-token: write\n", "")
        self.assertTrue(any("allowlist:" in x for x in lint_one(text, "gpu-vast-sweep.yml")))

    def test_gpu_job_with_id_token_read_fails(self):
        text = mutate(real("gpu-vast-sweep.yml"), "id-token: write", "id-token: read")
        self.assertTrue(lint_one(text, "gpu-vast-sweep.yml"))

    def test_widened_report_job_fails(self):
        text = real("gpu-vast.yml")
        i = text.index("  report:")
        anchor = "      statuses: write\n"
        j = text.index(anchor, i) + len(anchor)
        text = text[:j] + "      contents: write\n" + text[j:]
        out = lint_one(text, "gpu-vast.yml")
        self.assertTrue(any("`report`" in x for x in out), out)

    def test_widened_policy_job_fails(self):
        text = real("gpu-vast.yml")
        i = text.index("  policy:")
        anchor = "      pull-requests: read\n"
        j = text.index(anchor, i) + len(anchor)
        text = text[:j] + "      issues: write\n" + text[j:]
        self.assertTrue(any("`policy`" in x for x in lint_one(text, "gpu-vast.yml")))

    def test_new_job_must_be_added_deliberately(self):
        text = real("gpu-vast-sweep.yml") + "  extra:\n    runs-on: ubuntu-latest\n    permissions: {}\n    steps:\n      - run: true\n"
        self.assertTrue(any("`extra` is not in ALLOWLIST" in x for x in lint_one(text, "gpu-vast-sweep.yml")))

    def test_removed_job_is_reported(self):
        text = real("gpu-vast-sweep.yml").replace("  gpu:\n", "  gpu2:\n", 1)
        out = lint_one(text, "gpu-vast-sweep.yml")
        self.assertTrue(any("missing" in x for x in out))

    def test_other_workflows_are_not_held_to_the_allowlist(self):
        self.assertEqual(lint_one(GOOD, "other.yml"), [])


class Secrets(unittest.TestCase):
    SECRET = "      - run: echo\n        env:\n          X: ${{ secrets.VAST_BROKER_PATH }}\n"

    def test_secret_job_with_checkout_fails(self):
        text = GOOD + self.SECRET
        out = lint_one(text)
        self.assertTrue(any("secrets:" in x and "checkout" in x for x in out), out)

    def test_secret_job_with_any_third_party_action_fails(self):
        text = mutate(GOOD, f"actions/checkout@{SHA}", f"actions/cache@{SHA}") + self.SECRET
        self.assertTrue(any("secrets:" in x and "actions/cache" in x for x in lint_one(text)))

    def test_secret_job_with_only_run_steps_passes(self):
        text = mutate(GOOD, f"      - uses: actions/checkout@{SHA} # v4.4.0\n", "") + self.SECRET
        self.assertEqual(lint_one(text), [])

    def test_secret_in_other_job_is_not_blamed_on_checkout_job(self):
        text = GOOD + "  other:\n    runs-on: ubuntu-latest\n    steps:\n" + self.SECRET
        self.assertEqual(lint_one(text), [])

    def test_gpu_vast_environment_counts_as_touching(self):
        text = GOOD + "  env-job:\n    environment: gpu-vast\n    steps:\n      - uses: actions/checkout@" + SHA + "\n"
        self.assertTrue(any("secrets:" in x for x in lint_one(text)))

    def test_workflow_level_secret_taints_every_job(self):
        text = mutate(GOOD, "jobs:", "env:\n  A: ${{ secrets.VAST_BROKER_PATH }}\njobs:")
        self.assertTrue(any("secrets:" in x for x in lint_one(text)))

    def test_secrets_inherit_counts(self):
        text = GOOD + "  call:\n    uses: ./.github/workflows/w.yml\n    secrets: inherit\n"
        self.assertTrue(any("secrets:" in x for x in lint_one(text)))

    def test_comment_mentioning_secret_is_not_a_touch(self):
        text = mutate(GOOD, "      - run: echo hi", "      # VAST_BROKER_URL is not used here\n      - run: echo hi")
        self.assertEqual(lint_one(text), [])

    def test_real_gpu_job_has_no_actions(self):
        for name in ("gpu-vast.yml", "gpu-vast-sweep.yml"):
            wf = wl.parse(REPO / ".github" / "workflows" / name, real(name))
            gpu = [j for j in wf.jobs if j.name == "gpu"][0]
            self.assertTrue(wl.touches_secrets(gpu.lines))
            self.assertEqual(wl.uses_in(gpu.lines), [])


class Cli(unittest.TestCase):
    def test_exit_codes(self):
        d, root = tree(x__yml=GOOD)
        with d:
            self.assertEqual(wl.main(["--root", str(root)]), 0)
            (root / ".github" / "workflows" / "x.yml").write_text(mutate(GOOD, f"@{SHA}", "@v4"))
            self.assertEqual(wl.main(["--root", str(root)]), 1)

    def test_empty_tree_is_an_error(self):
        d, root = tree()
        with d:
            self.assertEqual(wl.main(["--root", str(root)]), 1)


if __name__ == "__main__":
    unittest.main()
