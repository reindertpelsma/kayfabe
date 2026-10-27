"""Freshness contracts extending C1's parser; temporary repositories, no GPU."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECKER = Path(__file__).with_name("doc_freshness.py")
TODAY = "2026-10-23"


class FreshnessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "-q")
        self.write("docs/ok.md", self.document())
        self.write("crates/kf-example/Cargo.toml", '[package]\nname = "kf-example"\n')
        self.write("crates/kf-example/src/lib.rs", "// KF3_ALIVE --alive-flag\n")
        self.git("add", ".")

    def git(self, *args):
        return subprocess.run(
            ["git", "-C", str(self.root), *args], check=True, capture_output=True, text=True
        ).stdout

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def document(self, state="LIVE", day=TODAY, extra="", body="Body."):
        return f"# Example\n\n**STATUS: {state}, {day}.** {extra}\n\n{body}\n"

    def run_check(self, expected=0, *args):
        run = subprocess.run(
            [
                sys.executable,
                str(CHECKER),
                "--root",
                str(self.root),
                "--today",
                TODAY,
                "--format",
                "json",
                *args,
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(run.returncode, expected, run.stderr or run.stdout)
        return json.loads(run.stdout)

    def kinds(self, expected=1, *args):
        return {item["kind"] for item in self.run_check(expected, *args)["violations"]}

    def test_status_is_required_outside_design(self):
        self.write("docs/guide.md", "# Guide\n\nAn undated guide.\n")
        self.assertIn("missing-status", self.kinds())

    def test_nested_markdown_requires_status(self):
        self.write("docs/reference/nested/guide.md", "# Guide\n")
        self.assertIn("missing-status", self.kinds())

    def test_architecture_requires_status(self):
        self.write("ARCHITECTURE.md", "# Architecture\n")
        self.assertIn("missing-status", self.kinds())

    def test_readme_requires_status_without_editing_it(self):
        self.write("README.md", "# Read me\n")
        self.assertIn("missing-status", self.kinds())

    def test_c1_metadata_forms_are_preserved(self):
        self.write(
            "docs/ok.md",
            "---\ntitle: Guide\n---\n\n# Guide\n\n" "> ### STATUS — 2026-10-23 / **LIVE**\n",
        )
        self.run_check()

    def test_status_buried_after_prose_fails(self):
        self.write("docs/ok.md", "# Guide\n\nProse.\n\nSTATUS: LIVE, 2026-10-23.\n")
        self.assertIn("misplaced-status", self.kinds())

    def test_status_with_invalid_calendar_date_fails(self):
        self.write("docs/ok.md", self.document(day="2026-02-30"))
        self.assertIn("invalid-date", self.kinds())

    def test_disallowed_status_fails(self):
        self.write("docs/ok.md", self.document(state="BUILT"))
        self.assertIn("invalid-status", self.kinds())

    def test_superseded_requires_linked_successor(self):
        self.write("docs/old.md", self.document(state="SUPERSEDED"))
        self.assertIn("missing-successor", self.kinds())

    def test_successor_must_exist(self):
        self.write("docs/old.md", self.document("SUPERSEDED", extra="[Next](missing.md)"))
        self.assertIn("broken-link", self.kinds())

    def test_external_successor_is_not_a_repository_successor(self):
        self.write("docs/old.md", self.document("SUPERSEDED", extra="[Next](https://example.org)"))
        self.assertIn("missing-successor", self.kinds())

    def test_self_successor_fails(self):
        self.write("docs/old.md", self.document("SUPERSEDED", extra="[Next](old.md)"))
        self.assertIn("missing-successor", self.kinds())

    def test_directory_successor_fails(self):
        self.write("docs/old.md", self.document("SUPERSEDED", extra="[Next](./)"))
        self.assertIn("missing-successor", self.kinds())

    def test_successor_in_status_is_reported(self):
        self.write("docs/old.md", self.document("SUPERSEDED", extra="[Next](ok.md)"))
        row = next(row for row in self.run_check()["documents"] if row["path"] == "docs/old.md")
        self.assertEqual(row["successor"], "docs/ok.md")

    def test_successor_may_point_into_read_only_archive(self):
        self.write("archive/docs/old.md", "# Preserved historical text\n")
        self.write(
            "docs/old.md", self.document("SUPERSEDED", extra="[Archive](../archive/docs/old.md)")
        )
        self.run_check()

    def test_old_live_with_dead_source_path_is_suspect(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`crates/gone/src/lib.rs`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_absolute_host_shell_path_is_not_a_repository_reference(self):
        host_path = "/scratch/session/launcher.sh"
        self.write("docs/ok.md", self.document(day="2026-01-01", body=f"`{host_path}`"))
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_absolute_host_python_path_is_omitted_from_report(self):
        host_path = "/opt/session/collector.py"
        self.write("docs/ok.md", self.document(day="2026-01-01", body=f"`{host_path}`"))
        run = subprocess.run(
            [
                sys.executable,
                str(CHECKER),
                "--root",
                str(self.root),
                "--today",
                TODAY,
                "--format",
                "markdown",
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(run.returncode, 0, run.stderr or run.stdout)
        self.assertNotIn(host_path, run.stdout)

    def test_absolute_source_filename_cannot_use_repository_suffix_alias(self):
        host_path = "/scratch/session/lib.rs"
        self.write("docs/ok.md", self.document(day="2026-01-01", body=f"`{host_path}`"))
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_broken_root_relative_markdown_link_still_fails(self):
        self.write("docs/ok.md", self.document(body="[code](/crates/gone/src/lib.rs)"))
        self.assertIn("broken-link", self.kinds())

    def test_old_live_with_missing_environment_variable_is_suspect(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`KF3_REMOVED_OPTION=1`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_old_live_with_missing_flag_is_suspect(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`--removed-option`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_old_live_with_missing_crate_is_suspect(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`kf-removed-crate`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_worktree_directory_is_not_a_crate_reference(self):
        self.write(
            "docs/ok.md",
            self.document(day="2026-01-01", body="Checkout: `/scratch/kf-old-worktree/`."),
        )
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_hyphenated_prose_is_not_a_crate_reference(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="kayfabe-owned resources."))
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_existing_command_filename_is_not_a_crate(self):
        self.write("scripts/kf-tool.py", "# command, not a package\n")
        self.git("add", ".")
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`scripts/kf-tool.py`"))
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_old_live_with_missing_branch_is_suspect(self):
        self.write(
            "docs/ok.md", self.document(day="2026-01-01", body="branch `codex/removed-branch`")
        )
        self.assertIn("stale-suspect", self.kinds())

    def test_explicit_short_branch_name_is_checked(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="Branch `v3-retired`."))
        self.assertIn("stale-suspect", self.kinds())

    def test_explicit_arbitrary_branch_name_is_checked(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="branch `retired-topic`."))
        self.assertIn("stale-suspect", self.kinds())

    def test_origin_prefixed_branch_is_checked(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`origin/codex/retired`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_age_alone_does_not_mean_stale(self):
        self.write("docs/ok.md", self.document(day="2026-01-01"))
        self.run_check()

    def test_recent_dead_references_are_reported_but_not_stale(self):
        self.write("docs/ok.md", self.document(body="`KF3_REMOVED_OPTION`"))
        result = self.run_check()
        self.assertIn("env:KF3_REMOVED_OPTION", result["documents"][0]["dead_references"])

    def test_configurable_age_boundary_is_strict(self):
        self.write("docs/ok.md", self.document(day="2026-08-24", body="`KF3_REMOVED_OPTION`"))
        self.run_check()
        self.assertIn("stale-suspect", self.kinds(1, "--max-age-days", "59"))

    def test_research_with_old_dead_references_is_not_stale(self):
        self.write(
            "docs/ok.md", self.document("RESEARCH", "2026-01-01", body="`KF3_REMOVED_OPTION`")
        )
        self.run_check()

    def test_live_references_are_searched_in_source(self):
        self.write(
            "docs/ok.md",
            self.document(
                day="2026-01-01",
                body="`KF3_ALIVE` `--alive-flag` `kf-example` `crates/kf-example/src/lib.rs`",
            ),
        )
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_doc_text_cannot_validate_its_own_missing_option(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`KF3_REMOVED_OPTION`"))
        self.write("archive/retired.rs", "// KF3_REMOVED_OPTION\n")
        self.write("traces/old.txt", "KF3_REMOVED_OPTION\n")
        self.git("add", ".")
        self.assertIn("stale-suspect", self.kinds())

    def test_reference_in_fenced_command_is_checked(self):
        self.write(
            "docs/ok.md", self.document(day="2026-01-01", body="```sh\nKF3_REMOVED_OPTION=1\n```")
        )
        self.assertIn("stale-suspect", self.kinds())

    def test_generic_environment_assignment_is_checked(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`REMOVED_SETTING=1`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_named_environment_variable_is_checked(self):
        self.write(
            "docs/ok.md",
            self.document(day="2026-01-01", body="environment variable `REMOVED_SETTING`"),
        )
        self.assertIn("stale-suspect", self.kinds())

    def test_root_source_path_cannot_resolve_inside_archive(self):
        self.write("archive/crates/gone/src/lib.rs", "// old implementation\n")
        self.git("add", ".")
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`crates/gone/src/lib.rs`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_root_source_path_cannot_resolve_to_unrelated_nested_path(self):
        self.write("tools/fixture/scripts/gone.py", "# unrelated fixture\n")
        self.git("add", ".")
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`scripts/gone.py`"))
        self.assertIn("stale-suspect", self.kinds())

    def test_existing_remote_branch_is_not_dead(self):
        self.git(
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.org",
            "commit",
            "-qm",
            "fixture",
        )
        self.git("update-ref", "refs/remotes/origin/codex/present", "HEAD")
        self.write("docs/ok.md", self.document(day="2026-01-01", body="branch `codex/present`"))
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_source_directory_named_integration_is_not_a_branch(self):
        self.write("scripts/integration/study.c", "// study\n")
        self.git("add", ".")
        self.write(
            "docs/ok.md", self.document(day="2026-01-01", body="`scripts/integration/study.c`")
        )
        self.assertEqual(self.run_check()["documents"][0]["dead_references"], [])

    def test_literal_relative_link_must_resolve(self):
        self.write("docs/ok.md", self.document(body="[missing](missing.md)"))
        self.assertIn("broken-link", self.kinds())

    def test_reference_link_must_resolve(self):
        self.write("docs/ok.md", self.document(body="[guide][next]\n\n[next]: missing.md"))
        self.assertIn("broken-link", self.kinds())

    def test_collapsed_reference_link_must_resolve(self):
        self.write("docs/ok.md", self.document(body="[next][]\n\n[next]: missing.md"))
        self.assertIn("broken-link", self.kinds())

    def test_image_link_must_resolve(self):
        self.write("docs/ok.md", self.document(body="![plot](missing.png)"))
        self.assertIn("broken-link", self.kinds())

    def test_html_relative_link_must_resolve(self):
        self.write("docs/ok.md", self.document(body='<a href="missing.md">guide</a>'))
        self.assertIn("broken-link", self.kinds())

    def test_encoded_space_and_parentheses_resolve(self):
        self.write("docs/a (b).md", self.document())
        self.write("docs/ok.md", self.document(body="[guide](a%20(b).md)"))
        self.run_check()

    def test_root_relative_link_resolves(self):
        self.write("docs/ok.md", self.document(body="[code](/crates/kf-example/src/lib.rs)"))
        self.run_check()

    def test_missing_heading_fragment_is_a_broken_link(self):
        self.write("docs/ok.md", self.document(body="[guide](ok.md#missing-section)"))
        self.assertIn("broken-link", self.kinds())

    def test_missing_same_doc_fragment_is_a_broken_link(self):
        self.write("docs/ok.md", self.document(body="[guide](#missing-section)"))
        self.assertIn("broken-link", self.kinds())

    def test_heading_fragment_and_duplicate_suffix_resolve(self):
        self.write(
            "docs/ok.md",
            self.document(
                body="## A `code` heading\n\n## A `code` heading\n\n"
                "[first](#a-code-heading) [second](#a-code-heading-1)"
            ),
        )
        self.run_check()

    def test_explicit_html_anchor_resolves(self):
        self.write(
            "docs/ok.md", self.document(body='<a id="stable-anchor"></a> [go](#stable-anchor)')
        )
        self.run_check()

    def test_setext_heading_fragment_resolves(self):
        self.write("docs/ok.md", self.document(body="Section\n-------\n\n[go](#section)"))
        self.run_check()

    def test_external_links_are_not_fetched(self):
        self.write(
            "docs/ok.md", self.document(body="[web](https://invalid.example/a) [mail](mailto:a@b)")
        )
        self.run_check()

    def test_link_examples_and_comments_are_not_live_links(self):
        self.write(
            "docs/ok.md",
            self.document(
                body="```md\n[example](missing.md)\n```\n\n<!-- [hidden](missing.md) -->"
            ),
        )
        self.run_check()

    def test_inline_code_link_example_is_not_a_live_link(self):
        self.write("docs/ok.md", self.document(body="Example: `[missing](missing.md)`."))
        self.run_check()

    def test_double_backtick_link_example_is_not_a_live_link(self):
        self.write("docs/ok.md", self.document(body="Example: ``[missing](missing.md)``."))
        self.run_check()

    def test_link_cannot_escape_checkout(self):
        self.write("docs/ok.md", self.document(body="[outside](../../outside.md)"))
        self.assertIn("broken-link", self.kinds())

    def test_readme_broken_link_is_checked(self):
        self.write("README.md", self.document(body="[missing](docs/missing.md)"))
        self.assertIn("broken-link", self.kinds())

    def test_archived_markdown_is_read_only_and_excluded(self):
        self.write("docs/archive/old.md", "# Old\n\n[missing](missing.md)\n")
        row = next(row for row in self.run_check()["documents"] if row["path"].endswith("old.md"))
        self.assertEqual(row["status"], "ARCHIVED")

    def test_nested_design_archive_is_excluded(self):
        original = "# Frozen design\n\n[old link](missing.md)\n"
        self.write("docs/design/archive/old.md", original)
        self.run_check()
        self.assertEqual((self.root / "docs/design/archive/old.md").read_text(), original)

    def test_non_markdown_assets_are_inventoried_without_status_edits(self):
        self.write("docs/reference/data.json", '{"historical": true}\n')
        row = next(
            row for row in self.run_check()["documents"] if row["path"].endswith("data.json")
        )
        self.assertEqual(row["status"], "ASSET")

    def test_published_manual_pages_are_inventoried_not_dated(self):
        self.write("docs/howto/page.md", "# Page\n\n[faq](FAQ.md)\n")
        self.write("docs/FAQ.md", "# FAQ\n\n[page](page.md)\n")
        result = self.run_check()
        rows = {row["path"]: row for row in result["documents"]}
        self.assertEqual(rows["docs/howto/page.md"]["status"], "MANUAL")
        self.assertEqual(rows["docs/FAQ.md"]["status"], "MANUAL")
        self.assertEqual(result["new"], 0)

    def test_evidence_log_status_word_is_not_document_metadata(self):
        self.write("docs/reference/run.log", "STATUS: invalid evidence output\n")
        row = next(row for row in self.run_check()["documents"] if row["path"].endswith("run.log"))
        self.assertEqual(row["status"], "ASSET")

    def test_legacy_baseline_is_explicit_and_green(self):
        self.write("docs/ok.md", "# Legacy\n")
        self.run_check(0, "--record")
        self.assertEqual(self.run_check()["new"], 0)

    def test_changed_legacy_doc_loses_exemption(self):
        self.write("docs/ok.md", "# Legacy\n")
        self.run_check(0, "--record")
        self.write("docs/ok.md", "# Legacy\n\nChanged prose.\n")
        self.assertEqual(self.run_check(1)["new"], 1)

    def test_changed_stale_doc_loses_exemption(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`KF3_REMOVED_OPTION`"))
        self.run_check(0, "--record")
        self.write(
            "docs/ok.md", self.document(day="2026-01-01", body="`KF3_REMOVED_OPTION` revised")
        )
        self.assertGreater(self.run_check(1)["new"], 0)

    def test_fixing_one_legacy_environment_reference_does_not_create_new_debt(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`KF3_GONE_A` `KF3_GONE_B`"))
        self.run_check(0, "--record")
        self.write("crates/kf-example/src/lib.rs", "// KF3_GONE_A\n")
        self.assertEqual(self.run_check()["new"], 0)

    def test_fixing_one_legacy_flag_reference_does_not_create_new_debt(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`--gone-a` `--gone-b`"))
        self.run_check(0, "--record")
        self.write("crates/kf-example/src/lib.rs", "// --gone-a\n")
        self.assertEqual(self.run_check()["new"], 0)

    def test_existing_stale_doc_fails_on_an_additional_dead_reference(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`KF3_GONE_A` `KF3_ALIVE`"))
        self.run_check(0, "--record")
        self.write("crates/kf-example/src/lib.rs", "// option removed\n")
        self.assertIn("stale-suspect", self.kinds())

    def test_removal_does_not_buy_new_debt(self):
        self.write("docs/old.md", "# Legacy\n")
        self.run_check(0, "--record")
        (self.root / "docs/old.md").unlink()
        self.write("docs/new.md", "# Legacy\n")
        self.assertEqual(self.run_check(1)["new"], 1)

    def test_newly_broken_target_fails_unchanged_document(self):
        self.write("docs/ok.md", self.document(body="[code](../crates/kf-example/src/lib.rs)"))
        self.run_check(0, "--record")
        (self.root / "crates/kf-example/src/lib.rs").unlink()
        self.assertIn("broken-link", self.kinds())

    def test_missing_source_after_baseline_is_new_stale_debt(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`KF3_ALIVE`"))
        self.run_check(0, "--record")
        self.write("crates/kf-example/src/lib.rs", "// option removed\n")
        self.assertIn("stale-suspect", self.kinds())

    def test_removed_flag_after_baseline_is_new_stale_debt(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`--alive-flag`"))
        self.run_check(0, "--record")
        self.write("crates/kf-example/src/lib.rs", "// flag removed\n")
        self.assertIn("stale-suspect", self.kinds())

    def test_removed_crate_after_baseline_is_new_stale_debt(self):
        self.write("docs/ok.md", self.document(day="2026-01-01", body="`kf-example`"))
        self.run_check(0, "--record")
        (self.root / "crates/kf-example/Cargo.toml").unlink()
        self.assertIn("stale-suspect", self.kinds())

    def test_removed_branch_after_baseline_is_new_stale_debt(self):
        self.git(
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.org",
            "commit",
            "-qm",
            "fixture",
        )
        self.git("update-ref", "refs/remotes/origin/codex/present", "HEAD")
        self.write("docs/ok.md", self.document(day="2026-01-01", body="branch `codex/present`"))
        self.run_check(0, "--record")
        self.git("update-ref", "-d", "refs/remotes/origin/codex/present")
        self.assertIn("stale-suspect", self.kinds())

    def test_empty_document_scan_is_infrastructure_failure(self):
        (self.root / "docs/ok.md").unlink()
        run = subprocess.run(
            [sys.executable, str(CHECKER), "--root", str(self.root)], capture_output=True, text=True
        )
        self.assertEqual(run.returncode, 2)

    def test_markdown_report_has_per_doc_evidence_and_decisions(self):
        run = subprocess.run(
            [
                sys.executable,
                str(CHECKER),
                "--root",
                str(self.root),
                "--today",
                TODAY,
                "--format",
                "markdown",
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        for required in (
            "STATUS:",
            TODAY,
            "docs/ok.md",
            "Age",
            "Dead references",
            "Successor",
            "INFERRED",
            "CODE READ",
        ):
            self.assertIn(required, run.stdout)


class WorkflowTests(unittest.TestCase):
    def workflow(self):
        return (CHECKER.parents[2] / ".github/workflows/doc-freshness.yml").read_text()

    def test_workflow_runs_on_docs_pull_requests_and_weekly(self):
        text = self.workflow()
        for required in (
            "pull_request:",
            "docs/**",
            "ARCHITECTURE.md",
            "README*",
            "schedule:",
            "23 5 * * 1",
            "workflow_dispatch:",
        ):
            self.assertIn(required, text)

    def test_workflow_generates_report_even_on_failure(self):
        text = self.workflow()
        for required in (
            "--format markdown",
            "if: always()",
            "actions/upload-artifact@v4",
            "fetch-depth: 0",
            "doc_freshness.py",
        ):
            self.assertIn(required, text)


if __name__ == "__main__":
    unittest.main()
