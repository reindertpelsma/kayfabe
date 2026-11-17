#!/usr/bin/env python3
"""Mutation checks of the GPU-use judge and its Linux wiring (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of scripts/bench/gpu_use.py, scripts/apps/gpu_use_rows.py, run_apps.sh, apps_hook.sh,
apps_matrix.sh, matrix_table.py or a row script; the named tests of test_gpu_use.py run against the copy and the failing ones are the killers. The
unmutated copy must be green first. The mutants are the ways this assertion could be quietly weakened: an empty census window that passes, a CPU device
name that is accepted, a guest that overrides the host, a row that drops its proof, a negative control that passes.

    python3 scripts/ci/gpu_use_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "gu": "scripts/bench/gpu_use.py",
    "rows": "scripts/apps/gpu_use_rows.py",
    "run": "scripts/apps/run_apps.sh",
    "hook": "scripts/apps/apps_hook.sh",
    "matrix": "scripts/apps/apps_matrix.sh",
    "mt": "scripts/apps/matrix_table.py",
    "blender": "scripts/apps/src/blender_render.py",
    "torch": "scripts/apps/src/torch_correct.py",
    "gen": "scripts/bench/windows/appmatrix/gen_apps.py",
    "eq": "scripts/bench/windows/appmatrix/guest/py/win_cuda_equiv.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_gpu_use."
COPY_DIRS = ["scripts/ci", "scripts/apps", "scripts/bench/windows/appmatrix"]
COPY_FILES = ["scripts/bench/gpu_use.py", "scripts/bench/windows/gpu_assert.py"]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""
CE, JC, AP, JU, NC, RW, CL, RR, SP, WI, SM, MT = (SUITE + n for n in (
    "Census.", "JudgeCensus.", "AppProofs.", "Judge.", "NegativeControls.", "Rows.", "Cli.", "RunAppsRows.", "ScriptProofs.", "Wiring.", "SmiJudge.", "MatrixTable."))
HK, WR = SUITE + "HookWithAFakeGuest.", SUITE + "WindowsRows."


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


BLENDER_PRINT = (
    'print("BLENDER_DEVICES dev=%s scene=%s active=%s cpu_enabled=%s enabled=%d names=%s" % (\n'
    '    dev, sc.cycles.device, active, any(d.type == "CPU" for d in enabled), len(enabled), "+".join(d.name for d in enabled)), flush=True)')
TORCH_PRINT = ('print("GPU_USE_PROOF torch device=%s name=%s peak_mib=%d" % (next(net_g.parameters()).device, torch.cuda.get_device_name(0), '
               'torch.cuda.max_memory_allocated() >> 20), flush=True)')
HOOK_ANNOTATE = 'else new=$(python3 -B -I "$GU" annotate "${gu_args[@]}") && [ -n "$new" ] && line=$new; fi'
HOOK_BASE = '  head -n "$q0" "$QLOG" 2>/dev/null | grep -a \'kf3: family=\' | tail -1 > "$OUT/$app.kf3.base"\n'
HOOK_SETTLE = ('  sleep "${APPS_KF3_SETTLE:-3}"   # the device prints its status line about every 2 s (on change): let the last one of this app land before the '
               'slice\n')
HASHCAT_NEW = "| tee $O/hashcat_full.txt | tail -30; grep -a -E '^(\\* Device #|CUDA API|OpenCL API|HIP API)' $O/hashcat_full.txt || true"
ENC_TAG = '; echo "ENCODER_TAG $(ffprobe -v error -select_streams v:0 -show_entries stream_tags=encoder -of default=nw=1:nk=1 $O/nvenc_h264.mp4)"'
CRASH_NOTE = 'out = rewrite(a.line, "NOTRUN", "gpu-use:NOT_RUN:judge-error " + clean(exc, 100)) + " gpu_use=NOT_RUN gpu_auth=none gpu_subs=- gpu_births=-"'
NEG_OK = 'gu.control_verdict(rec) if rec["verdict"] != "NA" else "CONTROL_BROKEN"'

MUTANTS = [
    # the census
    M("census-empty-window-passes", "gu", 'if c["status_lines"] == 0:\n        return NOT_RUN,', 'if c["status_lines"] == 0:\n        return PASS,',
      JC + "test_an_empty_slice_is_not_run_never_a_pass", RW + "test_a_pass_that_cannot_be_judged_becomes_notrun"),
    M("census-no-work-passes", "gu", 'return FAIL, ("host census: no GPU work', 'return PASS, ("host census: no GPU work',
      JC + "test_no_work_with_a_baseline_is_a_fail", JU + "test_the_host_saying_no_work_beats_a_guest_that_says_gpu",
      NC + "test_the_cpu_run_is_judged_fail_by_the_host_alone_when_the_guest_lies"),
    M("census-no-baseline-reads-as-no-work", "gu", '    if c["base_missing"]:\n        return NOT_RUN,', '    if False:\n        return NOT_RUN,',
      JC + "test_no_work_without_a_baseline_cannot_be_told_and_is_not_run", HK + "test_a_missing_baseline_is_not_run_and_never_a_pass"),
    M("census-floor-zero", "gu", "MIN_SUBMISSIONS = 32 ", "MIN_SUBMISSIONS = 0 ",
      JC + "test_the_floor_is_between_opening_the_device_and_a_cuda_context", JC + "test_no_work_with_a_baseline_is_a_fail"),
    M("census-retired-channel-counted-twice", "gu", "            live.pop(m.group(1).lower(), None)\n", "            pass\n",
      CE + "test_a_channel_retired_after_the_last_status_line_is_not_counted_twice", CE + "test_a_channel_retired_in_the_window_counts_only_its_window_part",
      CE + "test_one_of_two_live_channels_retiring_leaves_the_other_out_of_the_work"),
    M("census-baseline-ignored", "gu", 'live = dict(base["tokens"]) if base else {}', "live = {}",
      CE + "test_a_live_channel_the_window_did_not_touch_does_not_reduce_the_work", CE + "test_one_of_two_live_channels_retiring_leaves_the_other_out_of_the_work"),
    M("census-never-asked-of-a-guest-row", "gu", 'if need_census and side == "guest":', "if False:",
      NC + "test_the_cpu_run_is_judged_fail_by_the_host_alone_when_the_guest_lies", JU + "test_the_host_saying_no_work_beats_a_guest_that_says_gpu"),
    # the host engines
    M("smi-unreadable-passes", "gu", 'if rec.get("unreadable"):\n        return NOT_RUN,', 'if rec.get("unreadable"):\n        return PASS,',
      SM + "test_no_sample_at_all_is_unreadable_and_not_run", JU + "test_an_unreadable_sampler_is_not_run_not_idle"),
    M("smi-idle-passes", "gu", 'if rec.get("verdict") != PASS:\n        return FAIL,', 'if rec.get("verdict") != PASS:\n        return PASS,',
      JU + "test_an_idle_host_engine_fails_whatever_the_guest_printed", NC + "test_rows_with_engines_fail_on_an_idle_host_gpu_alone"),
    # the judge
    M("guest-overrides-the-host", "gu", "failed, not_run = FAIL in verdicts, NOT_RUN in verdicts",
      'failed, not_run = FAIL in verdicts and not (app and app["ok"] is True), NOT_RUN in verdicts',
      JU + "test_the_host_saying_no_work_beats_a_guest_that_says_gpu", JU + "test_an_idle_host_engine_fails_whatever_the_guest_printed",
      NC + "test_rows_with_engines_fail_on_an_idle_host_gpu_alone"),
    M("dropped-proof-passes", "gu", '            add(app_missing, "; ".join(app["reasons"]) or "no device proof", "app")',
      '            add(PASS, "; ".join(app["reasons"]) or "no device proof", "app")',
      JU + "test_a_dropped_proof_fails_and_an_unmeasured_marker_is_not_run", JU + "test_a_dropped_proof_fails_when_the_host_is_happy",
      RW + "test_a_row_whose_script_dropped_its_proof_is_rewritten_to_fail"),
    M("cpu-report-passes", "gu", '            add(FAIL, "; ".join(app["reasons"]) or "the application reports a non-GPU device", "app")',
      '            add(PASS, "; ".join(app["reasons"]) or "the application reports a non-GPU device", "app")',
      JU + "test_a_cpu_report_fails_even_when_the_host_saw_work", NC + "test_the_cpu_run_of_every_row_is_judged_fail_by_the_app_proof_alone"),
    M("cpu-device-name-accepted", "gu",
      'return bool(re.search(r"nvidia|geforce|rtx|quadro|tesla", str(name), re.I)) and not SOFT_GPU_NAMES.search(str(name))', "return True",
      AP + "test_every_row_has_a_gpu_log_that_passes_and_a_cpu_log_that_fails", AP + "test_blender_silent_fallback_is_caught_even_when_the_row_printed_ok",
      NC + "test_the_cpu_run_of_every_row_is_judged_fail_by_the_app_proof_alone"),
    # the application proofs
    M("llama-zero-offload-passes", "gu", '        if a == 0:\n            why.append("offloaded 0/%d layers: the model ran on the CPU" % b)\n        elif a != b:\n',
      '        if False:\n            why.append("x")\n        elif False:\n',
      AP + "test_llama_zero_of_n_is_the_cpu_message_that_still_matches_the_old_regexes", AP + "test_every_row_has_a_gpu_log_that_passes_and_a_cpu_log_that_fails",
      AP + "test_llama_partial_offload_is_not_a_pass"),
    M("llama-partial-offload-passes", "gu", "        elif a != b:\n", "        elif False:\n",
      AP + "test_llama_partial_offload_is_not_a_pass", NC + "test_llama_partial_offload_through_the_full_judge"),
    M("clpeak-cpu-block-passes", "gu", 'gpu = [b for b in blocks if _nvidia(b["name"]) and b["bw"]]', 'gpu = [b for b in blocks if b["bw"]]',
      AP + "test_clpeak_passes_on_the_nvidia_block_even_with_a_cpu_block_present_and_fails_on_the_cpu_block_alone",
      AP + "test_every_row_has_a_gpu_log_that_passes_and_a_cpu_log_that_fails"),
    M("hashcat-skipped-device-counts", "gu", 'used = [d for d in devs if d[2] != "skipped"]', "used = devs",
      AP + "test_hashcat_skipped_cpu_device_is_not_a_used_device", NC + "test_the_gpu_run_of_every_row_passes"),
    M("harness-header-is-application-output", "gu", 'if not l.startswith("=== "))', "if True)",
      AP + "test_the_harness_header_line_is_not_application_output", RR + "test_nvdec_decoder_pixel_format", NC + "test_the_gpu_run_of_every_row_passes"),
    # the rows module and the glue
    M("pass-not-rewritten", "rows", 'and rec["verdict"] in ("FAIL", "NOT_RUN"):', 'and rec["verdict"] in ():',
      RW + "test_a_pass_that_fell_back_becomes_a_fail_with_the_reason_in_the_note", CL + "test_annotate_through_the_cli_with_files",
      CL + "test_annotate_res_rewrites_a_res_file_once"),
    M("class-a-rows-judged", "rows", 'if cls not in ("b", "a+proof"):', "if False:",
      RW + "test_class_a_rows_are_recorded_never_judged", CL + "test_annotate_res_rewrites_a_res_file_once"),
    M("crashing-judge-leaves-the-pass", "rows", CRASH_NOTE, "out = a.line",
      CL + "test_a_crashing_judge_fails_closed_for_a_wired_pass", CL + "test_a_crash_inside_the_census_fails_closed_too"),
    M("negctl-control-always-ok", "rows", NEG_OK, '"CONTROL_OK"',
      CL + "test_negctl_prints_a_control_line", CL + "test_negctl_of_a_row_that_is_not_judged_is_a_broken_control",
      HK + "test_the_negative_control_writes_a_control_line_and_leaves_the_row_line_alone"),
    M("negctl-keeps-the-gpu-form", "run", 'if [ "${KF_NEGCTL:-0}" = 1 ]; then\n  export KF_BLENDER_DEVS=CPU',
      'if [ "${KF_NEGCTL:-0}" = 99 ]; then\n  export KF_BLENDER_DEVS=CPU',
      RR + "test_blender", RR + "test_llama", RR + "test_hashcat_keeps_its_device_header_beyond_the_tail"),
    M("negctl-blender-still-gpu", "run", "export KF_BLENDER_DEVS=CPU KF_NGL=0", "export KF_BLENDER_DEVS=CUDA KF_NGL=0",
      RR + "test_blender", SP + "test_the_negative_control_block_switches_every_class_b_row"),
    M("negctl-llama-still-offloads", "run", "KF_NGL=0 KF_HASHCAT_ARGS", "KF_NGL=99 KF_HASHCAT_ARGS",
      RR + "test_llama", RR + "test_llama_bench"),
    M("nvenc-drops-the-encoder-tag", "run", ENC_TAG, "",
      RR + "test_nvenc_encoder_tag", SP + "test_the_nvenc_and_hashcat_rows_keep_what_the_judge_reads"),
    M("hashcat-device-header-cut-off-again", "run", HASHCAT_NEW, "| tail -30",
      RR + "test_hashcat_keeps_its_device_header_beyond_the_tail", SP + "test_the_nvenc_and_hashcat_rows_keep_what_the_judge_reads"),
    M("hook-does-not-annotate", "hook", HOOK_ANNOTATE, "else :; fi",
      WI + "test_the_hook_rewrites_the_line_before_it_is_recorded_and_a_failed_judge_never_blanks_it",
      HK + "test_a_cpu_log_with_host_work_is_recorded_as_a_fail", HK + "test_a_gpu_run_is_recorded_as_a_pass_with_the_host_fields"),
    M("hook-baseline-line-dropped", "hook", HOOK_BASE, "",
      WI + "test_the_hook_samples_around_the_app_and_keeps_the_baseline_line", HK + "test_a_gpu_run_is_recorded_as_a_pass_with_the_host_fields",
      HK + "test_a_guest_that_prints_a_gpu_log_while_the_host_saw_nothing_is_recorded_as_a_fail"),
    M("hook-slices-before-the-last-status-line", "hook", HOOK_SETTLE, "",
      WI + "test_the_hook_waits_for_the_last_status_line_before_slicing", WI + "test_the_settle_default_is_longer_than_the_two_second_status_period"),
    M("hook-no-sampler", "hook", 'python3 -B -I "$GU" sample --out "$OUT" --app "$app" -- "$G"', '"$G"',
      WI + "test_the_hook_samples_around_the_app_and_keeps_the_baseline_line", HK + "test_the_box_nvidia_smi_is_sampled_around_the_app"),
    M("host-baseline-not-annotated", "matrix", 'annotate-res --side host --res "$R/$RES" --logs /opt/apps/out/host', 'annotate-res-disabled --side host --res "$R/$RES" --logs /opt/apps/out/host',
      WI + "test_the_host_baseline_is_annotated_after_the_run", WI + "test_the_host_annotation_follows_the_run_and_the_unlock"),
    M("notrun-counted-as-a-defect", "mt", "if unjudged and not (alone == PASS):", "if False:",
      MT + "test_a_guest_notrun_from_the_gpu_use_judge_is_unattributed_not_a_defect",
      MT + "test_an_isolated_gpu_use_notrun_after_a_batched_fail_is_still_unattributed"),
    # the row scripts
    M("blender-drops-the-devices-line", "blender", BLENDER_PRINT, "pass",
      SP + "test_blender_script_prints_the_devices_line_before_the_render", SP + "test_blender_devices_format_is_what_the_judge_reads",
      SP + "test_blender_script_in_gpu_mode_enables_only_the_gpu_device_and_prints_what_the_judge_reads"),
    M("blender-has-no-cpu-mode", "blender", 'cpu_mode = dev == "CPU"', "cpu_mode = False",
      SP + "test_blender_script_has_a_cpu_mode_for_the_negative_control", SP + "test_blender_script_in_cpu_mode_renders_on_the_cpu_and_the_judge_says_cpu"),
    M("torch-drops-the-proof", "torch", TORCH_PRINT, "pass",
      SP + "test_the_framework_scripts_print_the_proof_the_judge_reads", SP + "test_the_framework_proof_is_printed_after_the_work_not_before"),
    # the Windows rows (audit of 2026-11-17)
    M("windows-llama-gen-accepts-zero-offload", "gen", "out:offloaded ([1-9][0-9]*)/\\1 layers to GPU", "out:offloaded [0-9]+/[0-9]+ layers to GPU",
      WR + "test_llama_cuda_gen_needs_every_layer_offloaded", WR + "test_the_llama_rows_declare_the_exact_proofs"),
    M("windows-llama-bench-accepts-an-error-text", "gen", "out:\\| CUDA +\\| +[1-9][0-9]* \\|", "out:CUDA|NVIDIA",
      WR + "test_llama_cuda_bench_needs_the_cuda_backend_with_layers_offloaded", WR + "test_the_llama_rows_declare_the_exact_proofs"),
    M("windows-llama-vulkan-accepts-the-word", "gen", "out:ggml_vulkan: [0-9]+ = NVIDIA", "out:Vulkan|NVIDIA",
      WR + "test_llama_vulkan_bench_needs_an_nvidia_vulkan_device_not_just_the_word_vulkan", WR + "test_the_llama_rows_declare_the_exact_proofs"),
    M("windows-cpu-switch-open", "eq", 'if DEV != "cuda" and os.environ.get("KF_EQ_ALLOW_CPU") != "1":', "if False:",
      WR + "test_the_cuda_equivalence_script_refuses_a_cpu_run_without_the_second_switch", WR + "test_the_script_keeps_its_cpu_logic_test_behind_the_explicit_switch"),
]


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


def run_tests(root):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, SUITE.rstrip(".")], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=900)
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, wanted = mutant
    text = SOURCES[key]
    if text.count(old) != 1:
        return name, ["anchor x%d: %r" % (text.count(old), old[:50])], wanted
    root = tree(parent, "m", {key: text.replace(old, new)})
    killers = sorted(set(k.split(" (")[0] for k in run_tests(root)["killers"]))     # a failing subTest is the test
    shutil.rmtree(root, ignore_errors=True)
    return name, killers, wanted


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    chosen = [m for m in MUTANTS if a.only in m[0]]
    bad = 0
    with tempfile.TemporaryDirectory(prefix="kf-gpu-use-mutants-") as parent:
        base = run_tests(tree(parent, "base", {}))
        if base["killers"]:
            print("BASELINE RED: %s" % base["killers"], file=sys.stderr)
            return 2
        print("baseline green: %d tests" % base["tests"], flush=True)
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for name, killers, wanted in pool.map(lambda m: one(parent, m), chosen):
                short = {k.split(".", 1)[-1] for k in killers}
                named = [w for w in wanted if w.split(".", 1)[-1] in short or w in killers]
                ok = len(killers) >= 2 and len(named) >= 2 and not killers[0].startswith("anchor")      # two named killers for EVERY mutant, no exception
                print("%s %s: %d killers (%d named)" % ("PASS" if ok else "WEAK", name, len(killers), len(named)), flush=True)
                if not ok:
                    bad += 1
                    print("   killers: %s" % [k.split(".", 1)[-1] for k in killers[:12]], flush=True)
    print("%d/%d mutants killed" % (len(chosen) - bad, len(chosen)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
