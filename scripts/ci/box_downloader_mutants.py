#!/usr/bin/env python3
"""Mutation checks of the box downloader (2026-11-15): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of box_downloader.py, kf_fetch.py or vast_payload.py
(whole scripts/ci and scripts/kfci are copied, as instruction_mutants.py does: the payload tests import each other);
the named tests of test_box_downloader.py run against the copy and the failing ones are the killers. The unmutated
copy must be green first.

    python3 scripts/ci/box_downloader_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "dl": "scripts/ci/box_downloader.py",
    "fetch": "scripts/ci/kf_fetch.py",
    "payload": "scripts/ci/vast_payload.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_box_downloader."
COPY_DIRS = ["scripts/ci", "scripts/kfci"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""

MH, MF, FE, SR, BW, PS, WT, QU, PF, LP, WK = (SUITE + n for n in (
    "ManifestHostile.", "Fetching.", "Fetching.", "PrivateSources.", "Bandwidth.", "PrivateSources.", "Waiting.", "Quiet.",
    "Prefetch.", "LocalPins.", "WorkerLayer."))
SA = SUITE + "Standalone."


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    M("manifest-dotdot-dest-allowed", "dl", 'p in ("", ".", "..") or not SEG_RE.fullmatch(p)', "not SEG_RE.fullmatch(p)",
      MH + "test_traversal_dest_refused", MH + "test_error_messages_carry_no_values"),
    M("manifest-reserved-dest-allowed", "dl", 'return not parts[0] == STATE_DIR and not parts[-1].endswith(RESERVED_SUFFIXES)', "return True",
      MH + "test_reserved_suffix_dest_refused", MH + "test_state_directory_dest_refused"),
    M("manifest-duplicate-dest-allowed", "dl", "if folded == other or folded.startswith(other", "if False and folded.startswith(other",
      MH + "test_duplicate_dest_refused", MH + "test_dest_prefix_collision_refused"),
    M("manifest-duplicate-name-allowed", "dl", "        if name in names:\n", "        if False:\n",
      MH + "test_duplicate_name_refused", MH + "test_duplicate_name_with_other_pin_refused"),
    M("manifest-size-unbounded", "dl", '_int(raw["size"], path + ".size", 1, MAX_SIZE)', '_int(raw["size"], path + ".size", -2**80, 2**80)',
      MH + "test_negative_and_zero_sizes_refused", MH + "test_huge_sizes_refused"),
    M("manifest-bool-is-int", "dl", "if type(value) is not int or not lo <= value <= hi:", "if not isinstance(value, int) or not lo <= value <= hi:",
      MH + "test_bool_and_non_integer_sizes_refused", MH + "test_bool_priority_and_out_of_range_priority_refused"),
    M("manifest-private-after-mirror-allowed", "dl", 'if "mirror" in kinds and any(', 'if False and any(',
      MH + "test_private_source_after_mirror_refused", MH + "test_s3_after_mirror_refused"),
    M("manifest-http-mirror-allowed", "dl", "kf_fetch.valid_url(url, secure=not allow_http)\n    except kf_fetch.FetchError:\n        raise ManifestError(path + \".url\")",
      "kf_fetch.valid_url(url, secure=False)\n    except kf_fetch.FetchError:\n        raise ManifestError(path + \".url\")",
      MH + "test_http_mirror_refused_by_default", SA + "test_https_only_by_default"),
    M("manifest-unknown-keys-allowed", "dl", "if set(doc) - set(required) - set(optional) or any(k not in doc for k in required):",
      "if any(k not in doc for k in required):",
      MH + "test_unknown_keys_and_schema_refused", MH + "test_unknown_source_key_refused"),
    M("json-duplicate-keys-allowed", "dl", "        if k in out:\n            raise ManifestError(\"duplicate key\")", "        if False:\n            raise ManifestError(\"duplicate key\")",
      MH + "test_duplicate_json_key_and_nan_refused", PS + "test_urls_file_duplicate_key_refused"),
    M("order-ignores-priority", "dl", "key=lambda i: (index[i.needed_by], i.priority, order[i.name])", "key=lambda i: (index[i.needed_by], order[i.name])",
      MH + "test_ordering_priority_then_manifest_order_within_a_stage", FE + "test_order_is_stage_then_priority"),
    M("order-ignores-stage", "dl", "key=lambda i: (index[i.needed_by], i.priority, order[i.name])", "key=lambda i: (i.priority, order[i.name])",
      MH + "test_ordering_is_stage_first_then_priority", FE + "test_order_is_stage_then_priority"),
    # two layers (hash before the copy, hash of the bytes written): the mutant removes both
    M("pin-hash-unchecked", "dl", ["if hash_file(path, self.cancel_event) != item.sha256:\n                    continue",
                                   "if n != item.size or h.hexdigest() != item.sha256:"],
      ["if False:\n                    continue", "if False:"],
      LP + "test_pin_with_wrong_hash_is_ignored", LP + "test_wrong_hash_pin_is_never_published"),
    M("symlinks-never-detected", "dl", "return stat.S_ISLNK(os.lstat(path).st_mode)", "return False",
      FE + "test_symlink_dest_file_refused", FE + "test_symlink_directory_component_refused", FE + "test_symlink_unverified_refused"),
    M("derived-paths-not-checked", "dl", "                if _is_link(derived):\n", "                if False:\n",
      FE + "test_symlink_unverified_refused", FE + "test_symlink_state_file_refused"),
    M("no-source-fallback", "dl", "for index, source in enumerate(item.sources):", "for index, source in enumerate(item.sources[:1]):",
      FE + "test_fallback_to_second_mirror_never_mixes", PS + "test_missing_signed_url_falls_through_to_mirror", PS + "test_s3_without_credentials_is_skipped_not_fatal"),
    M("cancel-discards-partial", "dl", "                        if self.cancel_event.is_set():\n                            raise\n                        cls = e.kind",
      "                        cls = e.kind",
      SA + "test_cancel_keeps_partial_and_next_run_resumes", FE + "test_cancel_in_process_keeps_partial_and_reports_pending"),
    M("expired-url-not-requested-again", "dl", 'if e.kind == "AUTH" and attempt + 1 < rounds', "if False",
      PS + "test_expired_signed_url_is_requested_again", PS + "test_expired_s3_url_is_signed_again"),
    M("disk-reserve-ignored", "dl", "        return free >= need", "        return True",
      FE + "test_disk_reserve_refuses_before_any_request", FE + "test_reserve_bytes_are_added_to_the_need"),
    M("global-cap-ignored", "dl", "        self.global_bucket.acquire(n, self.cancel)\n", "        pass\n",
      BW + "test_global_cap_holds_across_parallel_items", BW + "test_throttle_object_applies_the_global_rate"),
    M("stream-cap-ignored", "dl", "        if self.stream_rate > 0:", "        if False:",
      BW + "test_per_stream_cap_holds_for_one_connection", BW + "test_throttle_object_applies_the_stream_rate_per_thread"),
    M("bucket-does-not-wait", "dl", "finish = start + n / self.rate", "finish = start",
      BW + "test_global_cap_holds_across_parallel_items", BW + "test_per_stream_cap_holds_for_one_connection", BW + "test_throttle_object_applies_the_global_rate"),
    M("workers-serialised", "dl", "max(1, min(self.jobs, len(self.queue) or 1))", "1",
      BW + "test_parallel_fetches_overlap", BW + "test_three_jobs_finish_in_about_one_transfer_time"),
    M("output-not-scrubbed", "dl", "self.emit_line(redact(line))", "self.emit_line(line)",
      QU + "test_emit_scrubs_even_if_a_url_slips_in", QU + "test_emit_scrubs_every_scheme"),
    M("redact-disabled", "dl", 'return URL_RE.sub("<url>", str(text))', "return str(text)",
      QU + "test_redact_scrubs_urls", QU + "test_emit_scrubs_every_scheme"),
    M("wait-ignores-failed", "dl", 'if any(r.get("state") == "failed" for r in rows):', "if False:",
      WT + "test_wait_returns_nonzero_when_failed", WT + "test_wait_one_failed_among_several_is_failure"),
    M("wait-trusts-status-without-file", "dl", 'if st is None or not stat.S_ISREG(st.st_mode) or st.st_size != items[n].get("size"):', "if False:",
      WT + "test_wait_refuses_a_verified_claim_without_the_file", WT + "test_wait_size_mismatch_between_status_and_file_is_failure"),
    M("wait-ignores-dead-downloader", "dl", 'if doc.get("state") in ("done", "failed", "cancelled") or (type(pid) is int and not _alive(pid)):', "if False:",
      WT + "test_wait_notices_a_dead_downloader", WT + "test_wait_cancelled_run_is_downloader_gone"),
    M("urls-file-permissions-ignored", "dl", "or st.st_mode & 0o077 or", "or",
      PS + "test_urls_file_permissions_enforced", PS + "test_s3_config_file_must_be_private_and_hold_known_keys"),
    M("urls-file-symlink-followed", "dl", "os.O_RDONLY | os.O_NOFOLLOW", "os.O_RDONLY",
      PS + "test_urls_file_symlink_and_foreign_schema_refused", PS + "test_urls_file_symlink_is_not_followed"),
    M("s3-token-dropped", "dl", "    if token:\n        query", "    if False:\n        query",
      PS + "test_s3_signature_matches_an_independent_reference", PS + "test_s3_presign_session_token_is_signed"),
    M("s3-key-derivation-wrong", "dl", 'key = ("AWS4" + secret_key).encode()', "key = secret_key.encode()",
      PS + "test_s3_presign_matches_the_aws_documented_vector", PS + "test_s3_signature_matches_an_independent_reference"),
    M("s3-endpoint-path-allowed", "dl", 'p.path not in ("", "/") or p.query', "p.query",
      PS + "test_s3_endpoint_with_path_or_http_refused", PS + "test_s3_endpoint_path_is_refused_not_signed"),
    M("worker-location-not-checked", "worker", "        return kf_fetch.valid_url(location, secure=not allow_http)", "        return location",
      WK + "test_worker_location_must_be_https_and_status_must_redirect", WK + "test_worker_location_with_credentials_refused"),
    M("kf-fetch-cancel-discards", "fetch", "            if self.cancelled:\n", "            if False:\n",
      SA + "test_cancel_keeps_partial_and_next_run_resumes", FE + "test_cancel_in_process_keeps_partial_and_reports_pending"),
    M("kf-fetch-throttle-hook-dropped", "fetch", "                        self.throttle(len(data))\n", "                        pass\n",
      BW + "test_global_cap_holds_across_parallel_items", BW + "test_per_stream_cap_holds_for_one_connection"),
    M("payload-prefetch-without-dry-run", "payload", "    if args.prefetch and not args.dry_run:", "    if False:",
      PF + "test_prefetch_without_dry_run_is_refused_not_wired", PF + "test_prefetch_without_dry_run_never_reads_the_manifest"),
    M("payload-prefetch-counts-all-items", "payload", "planned = [i for i in m.items if i.needed_by in plan]", "planned = list(m.items)",
      PF + "test_dry_run_prints_the_prefetch_summary_against_the_plan", PF + "test_items_of_a_planned_stage_are_counted"),
    M("payload-prefetch-line-dropped", "payload", "        if prefetch:\n            print(prefetch, flush=True)", "        pass",
      PF + "test_dry_run_prints_the_prefetch_summary_against_the_plan", PF + "test_items_of_a_planned_stage_are_counted"),
    M("payload-prefetch-bad-manifest-accepted", "payload", "    except box_downloader.ManifestError as error:", "    except ZeroDivisionError as error:",
      PF + "test_invalid_manifest_is_refused_and_value_not_echoed", PF + "test_unreadable_manifest_is_refused"),
]
FILES["worker"] = "scripts/ci/box_downloader_worker.py"
SOURCES["worker"] = (ROOT / FILES["worker"]).read_text()


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root, names):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, *names], cwd=root / "scripts/ci", capture_output=True, text=True,
                          timeout=1800, env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "PYTHONDONTWRITEBYTECODE": "1"})
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, tests = mutant
    text = SOURCES[key]
    pairs = list(zip(old, new)) if isinstance(old, list) else [(old, new)]
    for anchor, _ in pairs:
        if text.count(anchor) != 1:
            return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(anchor), anchor[:60])}
    for anchor, replacement in pairs:
        text = text.replace(anchor, replacement)
    root = tree(parent, "m", {key: text})
    killers = sorted(set(run_tests(root, tests)["killers"]))
    shutil.rmtree(root, ignore_errors=True)
    print("%s: %d named killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-downloader-mutants-") as parent:
        baseline = run_tests(tree(parent, "base", {}), [SUITE.rstrip(".")])
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
