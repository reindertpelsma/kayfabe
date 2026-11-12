#!/usr/bin/env python3
"""GPU-free fetch mutations: exact-once anchors, green baseline, >=2 named killers each."""
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
FETCH = "scripts/ci/kf_fetch.py"
SETUP = "scripts/ci/vast_setup.py"
BENCH = "scripts/bench/box/fetch_bench_data.sh"
CORE = "test_kf_fetch.FetchTests."
CACHE = "test_kf_fetch_integration.SetupIntegration."
MEDIA = "test_kf_fetch_integration.BenchIntegration."


def case(name, old, new, tests, path=FETCH):
    return name, path, old, new, tests


MUTANTS = [
    case("setup-default-is-parallel", 'os.environ.get("KF_FETCH_STREAMS", "1")', 'os.environ.get("KF_FETCH_STREAMS", "8")',
         [CACHE + "test_default_uses_established_opener_path", CACHE + "test_default_matches_explicit_one_request"], SETUP),
    case("setup-single-stream-bypassed", "if opener is None and streams != 1:", "if opener is None:",
         [CACHE + "test_single_stream_uses_established_opener_path", CACHE + "test_default_matches_explicit_one_request"], SETUP),
    case("media-default-is-parallel", "fetch_streams=${KF_FETCH_STREAMS:-1}", "fetch_streams=${KF_FETCH_STREAMS:-8}",
         [MEDIA + "test_file_mode_default_uses_exact_legacy_curl_command", MEDIA + "test_file_mode_default_matches_explicit_one_command"], BENCH),
    case("media-single-stream-bypassed", 'if [ "$fetch_streams" != 1 ]; then', "if true; then",
         [MEDIA + "test_file_mode_one_uses_exact_legacy_curl_command", MEDIA + "test_file_mode_default_matches_explicit_one_command"], BENCH),
    case("pinned-size-before-allocation-ignored", "if self.expected_size is not None and self.size != self.expected_size:", "if False:",
         [CORE + "test_larger_pinned_size_is_refused_before_allocation", CORE + "test_smaller_pinned_size_is_refused_before_allocation"]),
    case("setup-pinned-size-not-passed", '"--chunk-mb", "16", "--size", str(asset["size"])', '"--chunk-mb", "16"',
         [CACHE + "test_opt_in_uses_eight_streams_and_pinned_bytes", CACHE + "test_stream_override_is_honoured"], SETUP),
    case("media-stream-validation-skipped", '    [[ $fetch_streams =~ ^[1-9][0-9]{0,2}$ ]] && [ "$fetch_streams" -le 256 ] || die "KF_FETCH_STREAMS must be 1 through 256"', "    :",
         [MEDIA + "test_invalid_zero_streams_is_refused_before_mint", MEDIA + "test_excess_streams_is_refused_before_mint"], BENCH),
    case("redirect-origin-retried", "resolve = self.open if self.follow else self.retry_open", "resolve = self.retry_open",
         [CORE + "test_redirect_origin_500_does_not_spend_more_mints", CORE + "test_redirect_origin_reset_does_not_spend_more_mints"]),
    case("final-pin-ignored", "if digest_fd(self.fd, 0, self.size) != self.pin:", "if False:",
         [CORE + "test_corrupt_chunk_is_bad_hash_and_deleted", CORE + "test_wrong_pin_is_bad_hash_and_deleted"]),
    case("retries-disabled", "ATTEMPTS = 5", "ATTEMPTS = 1",
         [CORE + "test_flaky_500_then_reset_retries_each_chunk", CORE + "test_reset_retries_without_duplicating_output"]),
    case("range-offset-check-removed", '(self.ranges and response.headers.get("Content-Range") != expected_range)', "False",
         [CORE + "test_wrong_content_range_is_size_failure", CORE + "test_wrong_content_range_single_stream"]),
    case("fallback-keeps-parallel-stream-count", "            self.streams = 1\n", "            pass\n",
         [CORE + "test_no_range_falls_back_to_one_stream", CORE + "test_no_range_never_requests_chunk_offsets"]),
    case("redirects-implicitly-followed", "        return None\n\n\ndef valid_url", "        return urllib.request.HTTPRedirectHandler.redirect_request(self, *args, **kwargs)\n\n\ndef valid_url",
         [CORE + "test_redirect_requires_opt_in", CORE + "test_redirect_chain_refused"]),
    case("expiry-refresh-disabled", "REFRESHES = 2", "REFRESHES = 0",
         [CORE + "test_expiring_url_refresh_is_shared_by_workers", CORE + "test_expiring_url_refresh_single_stream"]),
    case("expiry-budget-raised", "REFRESHES = 2", "REFRESHES = 3",
         [CORE + "test_refresh_budget_is_two", CORE + "test_refresh_budget_single_stream"]),
    case("origin-credentials-forwarded", "{} if self.redirected else self.headers", "self.headers",
         [CORE + "test_stdin_url_and_origin_headers_never_forwarded", CORE + "test_worker_credentials_stay_on_origin_during_refresh"]),
    case("resume-bounds-ignored", "if not (0 <= start < end <= old_size):", "if False:",
         [CORE + "test_resume_truncated_file_rechecks_bounds", CORE + "test_resume_partly_truncated_chunk_is_refetched"]),
    case("resume-content-check-ignored", "if digest_fd(self.fd, start, end - start) == sha:", "if True:",
         [CORE + "test_resume_local_corruption_is_refetched", CORE + "test_resume_second_chunk_corruption_is_refetched"]),
    case("resume-etag-ignored", "for k, v in self.identity.items()):", 'for k, v in self.identity.items() if k != "etag"):',
         [CORE + "test_etag_change_restarts", CORE + "test_etag_disappearing_restarts"]),
    case("resume-size-ignored", "for k, v in self.identity.items()):", 'for k, v in self.identity.items() if k != "size"):',
         [CORE + "test_size_change_restarts", CORE + "test_size_change_from_smaller_object_restarts"]),
    case("completion-not-persisted", "                    self.save()\n                    self.progress()", "                    self.progress()",
         [CORE + "test_default_state_path", CORE + "test_resume_after_sigkill"]),
    case("chunk-offset-is-zero", "n = os.pwrite(fd, view, start)", "n = os.pwrite(fd, view, 0)",
         [CORE + "test_parallel_ranges_and_final_pin", CORE + "test_single_stream_ranges"]),
    case("short-pwrite-truncates-buffer", "        view = view[n:]", "        view = view[len(view):]",
         [CORE + "test_partial_pwrite_advances_offset", CORE + "test_one_byte_pwrite_keeps_all_bytes"]),
    case("signed-url-logged", "safe_label(label.hostname)", "self.remote.url",
         [CORE + "test_parallel_ranges_and_final_pin", CORE + "test_single_stream_ranges"]),
    case("setup-parallel-path-disabled", "if opener is None and streams != 1:", "if False:",
         [CACHE + "test_opt_in_uses_eight_streams_and_pinned_bytes", CACHE + "test_stream_override_is_honoured"], SETUP),
    case("setup-size-and-pin-check-removed", 'if temporary.stat().st_size != asset["size"] or digest(temporary) != asset["sha256"]:', "if False:",
         [CACHE + "test_successful_fetch_process_with_wrong_size_is_refused", CACHE + "test_successful_fetch_process_with_wrong_hash_is_refused"], SETUP),
    case("media-parallel-path-disabled", 'if [ "$fetch_streams" != 1 ]; then', "if false; then",
         [MEDIA + "test_file_mode_parallel_opt_in_verifies_and_renames", MEDIA + "test_file_mode_stream_override_reaches_fetcher"], BENCH),
    case("media-hash-failure-retried", '        if [ "$rc" -eq 3 ]; then', "        if false; then",
         [MEDIA + "test_file_mode_hash_failure_is_exit_three_without_retry", MEDIA + "test_file_mode_corrupt_chunk_is_exit_three_without_retry"], BENCH),
    case("writes-straight-to-final-path", 'return Path(str(out) + ".unverified")', "return Path(str(out))",
         [CORE + "test_final_path_absent_during_download_and_appears_whole", CORE + "test_final_path_never_written_while_partial_chunks_land"]),
    case("bad-final-file-left-after-hash-failure", 'print("FETCH_BAD_HASH", flush=True)', 'print("FETCH_BAD_HASH", flush=True); self.publish()',
         [CORE + "test_hash_failure_leaves_no_final_file", CORE + "test_wrong_pin_is_bad_hash_and_deleted"]),
    case("stale-final-file-kept", "self.out.unlink(missing_ok=True)\n            try:", "pass\n            try:",
         [CORE + "test_stale_final_file_is_removed_when_network_fails", CORE + "test_stale_final_file_is_removed_before_a_failed_hash"]),
    case("existing-match-shortcircuit-removed", "if self.existing_matches():", "if False:",
         [CORE + "test_existing_complete_file_short_circuits_without_network", CORE + "test_existing_complete_file_makes_no_request"]),
    case("unverified-kept-after-size-or-hash-failure", 'if e.kind in ("SIZE", "HASH") and not e.local:', "if False:",
         [CORE + "test_oversize_response_aborts_and_deletes_unverified", CORE + "test_wrong_pin_is_bad_hash_and_deleted"]),
    case("resume-target-ignored", 'for k, v in self.identity.items()):', 'for k, v in self.identity.items() if k != "target"):',
         [CORE + "test_state_for_another_target_is_stale", CORE + "test_state_without_target_from_old_layout_is_stale"]),
    case("fallback-mixes-sources", "            discard_partial(args.out, Path(args.state) if args.state is not None else Path(args.out + \".fetch-state.json\"))\n", "            pass\n",
         [CORE + "test_fallback_after_dropped_connection_never_mixes_sources", CORE + "test_fallback_refetches_every_chunk_after_partial_primary"]),
    case("fallback-only-after-network-class", "if args.fallback_url is None or e.local:", 'if args.fallback_url is None or e.local or e.kind != "NETWORK":',
         [CORE + "test_fallback_after_correct_headers_corrupt_bytes", CORE + "test_fallback_after_403"]),
    case("fallback-after-local-error", "if args.fallback_url is None or e.local:", "if args.fallback_url is None:",
         [CORE + "test_invalid_pin_is_local_and_does_not_fall_back", CORE + "test_invalid_stream_count_is_local_and_does_not_fall_back"]),
    case("stall-timeout-ignored", 'timeout=getattr(self, "timeout", 30))', "timeout=30)",
         [CORE + "test_fallback_after_stall_past_timeout", CORE + "test_stall_on_both_servers_fails_network"]),
    case("fallback-gets-worker-credentials", "download(args.fallback_url, False, {})", "download(args.fallback_url, False, headers)",
         [CORE + "test_fallback_never_receives_worker_credentials", CORE + "test_fallback_after_hash_failure_gets_no_credentials"]),
    case("fallback-follows-redirects", "download(args.fallback_url, False, {})", "download(args.fallback_url, args.follow_redirect, {})",
         [CORE + "test_fallback_does_not_follow_redirects", CORE + "test_fallback_redirect_after_hash_failure_is_refused"]),
]
COLLECT = '''import io,json,sys,unittest
suite=unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromName(n) for n in sys.argv[1:])
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({'tests':r.testsRun,'killers':sorted(set(t.id() for t,_ in r.failures+r.errors))}))
'''


def run(root, names):
    result = subprocess.run([sys.executable, "-c", COLLECT, *names], cwd=root / "scripts/ci",
                            capture_output=True, text=True, timeout=900,
                            env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})
    if result.returncode:
        raise RuntimeError("mutation harness failed")
    return json.loads(result.stdout.splitlines()[-1])


def prepare(parent):
    root = Path(tempfile.mkdtemp(prefix="fetch-", dir=parent))
    for rel in [FETCH, SETUP, BENCH, "scripts/ci/test_kf_fetch.py", "scripts/ci/test_kf_fetch_integration.py", "scripts/ci/test_vast_setup_cache.py"]:
        destination = root / rel
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, destination)
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    selected = [m for m in MUTANTS if args.only in m[0]]
    if not selected or not 1 <= args.jobs <= 2:
        parser.error("select mutants and use one or two jobs")
    with tempfile.TemporaryDirectory(prefix="kf-fetch-mutants-") as parent:
        base = run(prepare(parent), ["test_kf_fetch", "test_kf_fetch_integration"])
        if base["killers"]:
            print(json.dumps({"baseline": base}, indent=2))
            return 1

        def one(mutant):
            name, rel, old, new, names = mutant
            root = prepare(parent)
            path = root / rel
            source = path.read_text()
            if source.count(old) != 1:
                return {"mutant": name, "killers": [], "error": "anchor not exact-once"}
            path.write_text(source.replace(old, new))
            result = run(root, names)
            print(f"{name}: {len(result['killers'])} named killers", file=sys.stderr, flush=True)
            return {"mutant": name, **result}

        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            rows = list(pool.map(one, selected))
    print(json.dumps({"baseline": base, "mutants": rows}, indent=2))
    return int(any(len(row["killers"]) < 2 for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
