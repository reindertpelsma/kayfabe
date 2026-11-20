#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_config.py mutations (2026-11-19): each is one exact-once text replacement in a private copy of the module, and BOTH named tests of
test_propimage_config.py must fail on it (the repository's mutant rule: two named killers per mutant). The baseline must pass first."""
import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT / "scripts/ci"
SRC = (HERE / "propimage_config.py").read_text()
TEST = HERE / "test_propimage_config.py"
V = "test_propimage_config."


def case(name, old, new, *tests):
    edits = old if type(old) is list else [(old, new)]
    text = SRC
    for o, n in edits:
        assert text.count(o) == 1, (name, o, text.count(o))
        text = text.replace(o, n)
    assert len(set(tests)) >= 2, name
    return name, text, tests


CASES = [
    # ---- the document
    case("schema-name-not-checked", 'type(doc) is dict and doc.get("schema") == SCHEMA, "SCHEMA")', 'type(doc) is dict, "SCHEMA")',
         "test_wrong_schema_is_refused", "test_missing_schema_and_non_object_are_refused"),
    case("top-level-typo-allowed", 'set(doc) <= {"schema", "status", "bundles"} and "bundles" in doc, "TOP_KEY")', '"bundles" in doc, "TOP_KEY")',
         "test_unknown_top_level_key_is_refused", "test_a_typo_in_the_file_is_refused_by_the_cli"),
    case("bundles-key-optional", '{"schema", "status", "bundles"} and "bundles" in doc, "TOP_KEY")', '{"schema", "status", "bundles"}, "TOP_KEY")',
         "test_missing_bundles_is_refused", "test_a_file_without_bundles_is_refused_by_the_cli"),
    case("status-unbounded", 'len(doc["status"]) <= 4000', 'True', "test_status_must_be_a_bounded_string", "test_a_status_of_4001_characters_is_refused_by_the_cli"),
    case("empty-bundle-table-allowed", '0 < len(bundles) <= MAX_BUNDLES, "BUNDLES")', '0 <= len(bundles) <= MAX_BUNDLES, "BUNDLES")',
         "test_empty_bundle_table_is_refused", "test_a_file_with_no_bundles_is_refused_by_the_cli"),
    case("bundle-count-unbounded", '0 < len(bundles) <= MAX_BUNDLES, "BUNDLES")', '0 < len(bundles), "BUNDLES")',
         "test_too_many_bundles_are_refused", "test_seventeen_bundles_are_refused_by_the_cli"),
    case("duplicate-object-key-allowed", 'refuse(len(set(keys)) == len(keys), "DUPLICATE_R2")', 'pass',
         "test_duplicate_object_keys_are_refused", "test_duplicate_object_keys_across_platforms_are_refused"),
    case("duplicate-json-key-allowed", '        if k in out:\n            raise ConfigError("DUPLICATE_KEY:" + str(k)[:32])\n', '',
         "test_duplicate_object_key_in_the_text_is_refused", "test_duplicate_key_inside_a_bundle_is_refused"),
    case("json-error-escapes", '    except json.JSONDecodeError:\n        raise ConfigError("JSON") from None', '    except json.JSONDecodeError:\n        raise',
         "test_invalid_json_is_a_named_refusal", "test_invalid_json_exits_2"),
    # ---- a bundle
    case("bundle-id-unchecked", 'type(bid) is str and BUNDLE_ID_RE.fullmatch(bid), "BUNDLE_ID:"', 'type(bid) is str, "BUNDLE_ID:"',
         "test_bundle_id_is_a_lower_case_token", "test_a_bundle_id_with_a_space_is_refused_by_the_cli"),
    case("bundle-shape-unchecked", 'refuse(type(b) is dict, "BUNDLE_SHAPE:" + bid)', 'pass',
         "test_bundle_must_be_an_object", "test_a_bundle_that_is_a_string_is_refused_by_the_cli"),
    case("bundle-typo-allowed", 'refuse(set(b) <= BUNDLE_KEYS, "BUNDLE_KEY:" + bid)', 'pass',
         "test_unknown_bundle_key_is_refused", "test_a_typo_in_a_bundle_is_refused_by_the_cli"),
    case("required-bundle-key-optional", 'refuse(all(k in b for k in BUNDLE_REQUIRED), "BUNDLE_MISSING:" + bid)', 'pass',
         "test_every_required_bundle_key_is_required", "test_a_bundle_without_apps_is_refused_by_the_cli"),
    case("os-unchecked", 'refuse(b["os"] in OSES, "BUNDLE_OS:" + bid)', 'pass', "test_os_is_windows_or_linux", "test_parse_validates"),
    case("object-key-shape-unchecked", 'type(b["r2"]) is str and KEY_RE.fullmatch(b["r2"]), "BUNDLE_R2:" + bid)\n    refuse(".."',
         'type(b["r2"]) is str, "BUNDLE_R2:" + bid)\n    refuse(".."', "test_object_key_shapes", "test_an_absolute_object_key_is_refused"),
    case("object-key-climb-allowed", '".." not in b["r2"].split("/") and "//" not in b["r2"] and not b["r2"].endswith("/"), "BUNDLE_R2:" + bid)',
         '"//" not in b["r2"] and not b["r2"].endswith("/"), "BUNDLE_R2:" + bid)',
         "test_object_key_may_not_climb", "test_object_key_climb_in_the_last_segment_is_refused"),
    case("object-key-double-slash-allowed", '".." not in b["r2"].split("/") and "//" not in b["r2"] and not b["r2"].endswith("/"), "BUNDLE_R2:" + bid)',
         '".." not in b["r2"].split("/") and not b["r2"].endswith("/"), "BUNDLE_R2:" + bid)',
         "test_object_key_shapes", "test_object_key_with_a_double_slash_is_refused"),
    case("object-key-trailing-slash-allowed", '".." not in b["r2"].split("/") and "//" not in b["r2"] and not b["r2"].endswith("/"), "BUNDLE_R2:" + bid)',
         '".." not in b["r2"].split("/") and "//" not in b["r2"], "BUNDLE_R2:" + bid)',
         "test_object_key_shapes", "test_object_key_with_a_trailing_slash_is_refused"),
    case("size-zero-allowed", '0 < b["size_bytes"] < MAX_SIZE', '0 <= b["size_bytes"] < MAX_SIZE', "test_size_is_a_positive_bounded_integer", "test_a_zero_size_is_refused_by_the_cli"),
    case("size-unbounded", '0 < b["size_bytes"] < MAX_SIZE', '0 < b["size_bytes"]', "test_size_is_a_positive_bounded_integer", "test_a_size_of_the_limit_is_refused_by_the_cli"),
    case("size-bool-allowed", 'return type(value) is int', 'return isinstance(value, int)', "test_size_is_a_positive_bounded_integer", "test_a_true_size_is_refused_by_the_cli"),
    case("apps-may-be-empty", 'refuse(type(value) is list and 0 < len(value) <= limit, reason)', 'refuse(type(value) is list and len(value) <= limit, reason)',
         "test_apps_are_a_non_empty_list_of_unique_tokens", "test_rows_are_unique_tokens"),
    case("apps-unbounded", 'refuse(type(value) is list and 0 < len(value) <= limit, reason)', 'refuse(type(value) is list and 0 < len(value), reason)',
         "test_apps_are_a_non_empty_list_of_unique_tokens", "test_a_bundle_with_sixty_five_apps_is_refused_by_the_cli"),
    case("apps-names-unchecked", 'refuse(all(type(x) is str and regex.fullmatch(x) for x in value), reason)', 'pass',
         "test_apps_are_a_non_empty_list_of_unique_tokens", "test_rows_are_unique_tokens"),
    case("apps-duplicates-allowed", 'refuse(len(set(value)) == len(value), reason)', 'pass',
         "test_apps_are_a_non_empty_list_of_unique_tokens", "test_solo_rows_are_unique_tokens"),
    case("sha256-unchecked", 'type(b["sha256"]) is str and HEX64.fullmatch(b["sha256"]), "BUNDLE_SHA256:"', 'True, "BUNDLE_SHA256:"',
         "test_sha256_is_64_lower_case_hex", "test_an_uppercase_sha256_is_refused_by_the_cli"),
    case("ref-unchecked", 'type(b["ref"]) is str and REF_RE.fullmatch(b["ref"]), "BUNDLE_REF:"', 'True, "BUNDLE_REF:"',
         "test_ref_is_a_pins_name", "test_a_ref_with_a_slash_is_refused_by_the_cli"),
    case("families-unknown-allowed", 'all(f in FAMILIES for f in fam) and len(set(fam)) == len(fam)', 'len(set(fam)) == len(fam)',
         "test_gpu_families_are_known_and_unique", "test_an_unknown_family_is_refused_by_the_cli"),
    case("families-duplicates-allowed", 'all(f in FAMILIES for f in fam) and len(set(fam)) == len(fam)', 'all(f in FAMILIES for f in fam)',
         "test_gpu_families_are_known_and_unique", "test_a_repeated_family_is_refused_by_the_cli"),
    case("families-empty-allowed", 'type(fam) is list and fam and all(', 'type(fam) is list and all(',
         "test_gpu_families_are_known_and_unique", "test_an_empty_family_list_is_refused_by_the_cli"),
    # ---- secrets in the notes
    case("email-not-secretish", 'SECRETISH = re.compile(r"@|[A-Za-z0-9]{24,}")', 'SECRETISH = re.compile(r"[A-Za-z0-9]{24,}")',
         "test_licence_note_with_an_email_address_is_refused", "test_basis_with_an_email_address_is_refused"),
    case("token-not-secretish", 'SECRETISH = re.compile(r"@|[A-Za-z0-9]{24,}")', 'SECRETISH = re.compile(r"@|[A-Za-z0-9]{300,}")',
         "test_licence_note_with_a_key_sized_token_is_refused", "test_basis_with_a_sha256_sized_token_is_refused"),
    case("token-threshold-too-low", 'SECRETISH = re.compile(r"@|[A-Za-z0-9]{24,}")', 'SECRETISH = re.compile(r"@|[A-Za-z0-9]{8,}")',
         "test_a_23_character_word_is_not_a_token", "test_a_plain_note_is_accepted"),
    case("secretish-check-dropped", 'refuse(not SECRETISH.search(value), "%s_SECRETISH:%s" % (name, bid))', 'pass',
         "test_licence_note_with_an_email_address_is_refused", "test_basis_with_a_sha256_sized_token_is_refused"),
    case("note-shape-unchecked", 'refuse(type(value) is str and TEXT_RE.fullmatch(value), "%s:%s" % (name, bid))', 'pass',
         "test_notes_are_printable_bounded_text", "test_a_note_with_a_control_character_is_refused_by_the_cli"),
    # ---- the windows run
    case("run-shape-unchecked", 'refuse(type(b["run"]) is dict, "RUN_SHAPE:" + bid)', 'pass', "test_run_must_be_an_object", "test_a_run_that_is_a_list_is_refused_by_the_cli"),
    case("run-typo-allowed", 'refuse(set(run) <= WINDOWS_RUN_KEYS, "RUN_KEY:" + bid)', 'pass', "test_unknown_run_key_is_refused", "test_linux_key_on_a_windows_run_is_refused"),
    case("run-required-optional", 'refuse(all(k in run for k in WINDOWS_RUN_REQUIRED), "RUN_MISSING:" + bid)', 'pass',
         "test_rows_file_and_pins_file_are_required", "test_a_windows_run_without_a_pins_file_is_refused_by_the_cli"),
    case("rows-file-unchecked", '_relative_path(run["rows_file"], "RUN_ROWS_FILE:" + bid)\n    _relative_path(run["pins_file"], "RUN_PINS_FILE:" + bid)\n    rows',
         '_relative_path(run["pins_file"], "RUN_PINS_FILE:" + bid)\n    rows', "test_paths_are_relative_and_do_not_climb", "test_a_climbing_rows_file_is_refused_by_the_cli"),
    case("pins-file-unchecked", '_relative_path(run["rows_file"], "RUN_ROWS_FILE:" + bid)\n    _relative_path(run["pins_file"], "RUN_PINS_FILE:" + bid)\n    rows',
         '_relative_path(run["rows_file"], "RUN_ROWS_FILE:" + bid)\n    rows', "test_paths_are_relative_and_do_not_climb", "test_an_absolute_pins_file_is_refused_by_the_cli"),
    case("path-climb-allowed", 'refuse(".." not in value.split("/") and "//" not in value and not value.endswith("/"), reason)',
         'refuse("//" not in value and not value.endswith("/"), reason)', "test_paths_are_relative_and_do_not_climb", "test_a_climbing_rows_file_is_refused_by_the_cli"),
    case("path-double-slash-allowed", 'refuse(".." not in value.split("/") and "//" not in value and not value.endswith("/"), reason)',
         'refuse(".." not in value.split("/") and not value.endswith("/"), reason)', "test_paths_are_relative_and_do_not_climb", "test_a_double_slash_in_a_path_is_refused"),
    case("rows-unchecked", '_unique_strings(rows, ROW_RE, "RUN_ROWS:" + bid, 128)', 'pass', "test_rows_are_unique_tokens", "test_a_run_with_an_uppercase_row_is_refused_by_the_cli"),
    case("solo-rows-unchecked", '_unique_strings(run["solo_rows"], ROW_RE, "RUN_SOLO_ROWS:" + bid, 128)', 'pass', "test_solo_rows_are_unique_tokens", "test_a_run_with_a_repeated_solo_row_is_refused_by_the_cli"),
    case("solo-rows-need-not-be-rows", 'refuse(set(run["solo_rows"]) <= set(rows), "RUN_SOLO_ROWS_UNKNOWN:" + bid)', 'pass',
         "test_solo_rows_must_be_rows_when_rows_are_listed", "test_a_solo_row_that_is_not_a_row_is_refused_by_the_cli"),
    case("data-iso-not-boolean", 'refuse(type(run["data_iso"]) is bool, "RUN_DATA_ISO:" + bid)', 'pass', "test_data_iso_and_audio_are_booleans", "test_data_iso_one_is_refused_by_the_cli"),
    case("audio-not-boolean", 'refuse(type(run["audio"]) is bool, "RUN_AUDIO:" + bid)', 'pass', "test_data_iso_and_audio_are_booleans", "test_audio_yes_is_refused_by_the_cli"),
    case("guest-ram-unchecked", '(value == 0 or (1024 <= value <= 1 << 20 and value % 1024 == 0))', 'True', "test_guest_ram_is_zero_or_a_whole_number_of_gib", "test_guest_ram_of_1500_is_refused_by_the_cli"),
    case("guest-ram-fraction-of-gib-allowed", 'and value % 1024 == 0)), "RUN_GUEST_RAM:"', ')), "RUN_GUEST_RAM:"', "test_guest_ram_is_zero_or_a_whole_number_of_gib", "test_guest_ram_of_1500_is_refused_by_the_cli"),
    case("guest-ram-unbounded", '1024 <= value <= 1 << 20 and', '1024 <= value and', "test_guest_ram_is_zero_or_a_whole_number_of_gib", "test_a_guest_ram_of_two_tib_is_refused_by_the_cli"),
    case("args-keys-open", 'refuse(type(args) is dict and set(args) <= ARG_KEYS, "ARGS_KEY:" + bid)', 'refuse(type(args) is dict, "ARGS_KEY:" + bid)',
         "test_harness_args_keys_are_fixed", "test_a_typo_in_harness_args_is_refused_by_the_cli"),
    case("args-words-unchecked", 'all(type(w) is str and WORD_RE.fullmatch(w) for w in value), "ARGS:', 'True, "ARGS:',
         "test_harness_words_are_short_plain_tokens", "test_per_row_words_are_checked_like_the_others"),
    case("args-word-count-unbounded", 'type(value) is list and len(value) <= 16 and', 'type(value) is list and', "test_harness_words_are_short_plain_tokens", "test_a_run_with_seventeen_words_is_refused_by_the_cli"),
    case("shell-metacharacters-in-words", 'WORD_RE = re.compile(r"[A-Za-z0-9._:=/-]{1,64}")', 'WORD_RE = re.compile(r"[^\\x00]{1,64}")',
         "test_harness_words_are_short_plain_tokens", "test_per_row_words_are_checked_like_the_others"),
    case("per-row-type-unchecked", 'refuse(type(per_row) is dict, "ARGS_PER_ROW:" + bid)', 'pass', "test_per_row_words_are_checked_like_the_others", "test_per_row_that_is_a_list_is_refused_by_the_cli"),
    case("per-row-name-unchecked", 'refuse(type(row) is str and ROW_RE.fullmatch(row), "ARGS_PER_ROW_NAME:" + bid)', 'pass',
         "test_per_row_words_are_checked_like_the_others", "test_per_row_name_with_a_space_is_refused_by_the_cli"),
    case("per-row-must-be-row-dropped", 'refuse(row in rows, "ARGS_PER_ROW_UNKNOWN:%s:%s" % (bid, row))', 'pass',
         "test_per_row_names_must_be_rows_when_rows_are_listed", "test_per_row_name_outside_rows_is_refused_by_the_cli"),
    case("data-iso-word-allowed-before", 'refuse("--data-iso" not in [w for where in ("before_apps", "after_apps") for w in args.get(where, [])]\n               and not any(',
         'refuse(True\n               and not any(', "test_the_data_volume_flag_is_set_by_the_field_not_by_words", "test_the_data_volume_flag_after_the_apps_is_refused"),
    case("data-iso-word-allowed-per-row", 'and not any("--data-iso" in words for words in per_row.values()), "ARGS_DATA_ISO_SET_BY_FIELD:"', 'and True, "ARGS_DATA_ISO_SET_BY_FIELD:"',
         "test_the_data_volume_flag_is_set_by_the_field_not_by_words", "test_the_data_volume_flag_in_a_per_row_list_is_refused"),
    # ---- the linux run
    case("linux-run-typo-allowed", 'refuse(set(run) <= LINUX_RUN_KEYS, "RUN_KEY:" + bid)', 'pass', "test_windows_keys_on_a_linux_run_are_refused", "test_linux_run_with_a_typo_is_refused_by_the_cli"),
    case("linux-run-required-optional", 'refuse(all(k in run for k in LINUX_RUN_REQUIRED), "RUN_MISSING:" + bid)', 'pass',
         "test_all_three_linux_keys_are_required", "test_a_linux_run_without_isolation_is_refused_by_the_cli"),
    case("linux-paths-unchecked", '_relative_path(run["rows_file"], "RUN_ROWS_FILE:" + bid)\n    _relative_path(run["pins_file"], "RUN_PINS_FILE:" + bid)\n    refuse(run["isolation"]',
         'refuse(run["isolation"]', "test_linux_paths_are_checked", "test_a_linux_run_with_a_climbing_path_is_refused_by_the_cli"),
    case("isolation-unchecked", 'refuse(run["isolation"] in ISOLATIONS, "RUN_ISOLATION:" + bid)', 'pass', "test_isolation_is_row_or_shared", "test_a_linux_run_with_isolation_vm_is_refused_by_the_cli"),
    case("os-picks-the-wrong-run-check", 'if b["os"] == "windows":\n            check_windows_run(bid, b["run"])', 'if b["os"] == "linux":\n            check_windows_run(bid, b["run"])',
         "test_full_run_is_accepted", "test_linux_run_is_accepted"),
    # ---- the resolver
    case("resolver-skips-unknown-apps", '        refuse(have, "UNKNOWN_APP:" + name)\n', '        if not have:\n            continue\n',
         "test_unknown_app_is_refused_by_name", "test_unknown_app_among_known_ones_is_still_refused"),
    case("resolver-unknown-app-order", 'wanted = sorted(set(apps))', 'wanted = list(dict.fromkeys(apps))', "test_the_first_unknown_app_in_name_order_is_the_one_named", "test_the_unknown_app_named_does_not_depend_on_the_request_order"),
    case("resolver-bad-name-accepted", '        refuse(APP_RE.fullmatch(name), "BAD_APP:" + name[:32])\n', '        pass\n', "test_bad_app_name_is_refused_by_name", "test_cli_refuses_a_bad_app_name"),
    case("resolver-request-shape-unchecked", 'refuse(type(apps) in (list, tuple, set, frozenset) and all(type(a) is str for a in apps), "APPS_SHAPE")', 'pass',
         "test_request_shape_is_checked", "test_a_bare_string_is_not_a_request"),
    case("resolver-os-unchecked", 'refuse(os is None or os in OSES, "BUNDLE_OS:" + str(os)[:16])', 'pass', "test_the_os_argument_is_checked", "test_an_unknown_os_is_refused_even_for_an_empty_request"),
    case("resolver-ignores-os", 'and (os is None or b["os"] == os))', 'and True)', "test_the_os_argument_hides_the_other_platform", "test_the_os_argument_settles_an_ambiguous_app"),
    case("resolver-ambiguity-allowed", '        refuse(len({bundles[bid]["os"] for bid in have}) == 1, "AMBIGUOUS_APP:" + name)\n', '',
         "test_an_app_on_both_platforms_is_ambiguous_without_an_os", "test_cli_refuses_an_ambiguous_app"),
    case("resolver-empty-request-crashes", '    if not wanted:\n        return []\n', '', "test_empty_request_needs_no_bundle", "test_resolve_of_an_empty_list_is_empty"),
    case("resolver-first-cover-wins", 'key = (sum(bundles[bid]["size_bytes"] for bid in pick), count, pick)', 'key = (0, 0, pick)',
         "test_fewest_bytes_beat_fewest_bundles", "test_the_cheaper_bundle_wins_even_with_a_later_id"),
    case("resolver-fewest-bundles-first", 'key = (sum(bundles[bid]["size_bytes"] for bid in pick), count, pick)', 'key = (count, sum(bundles[bid]["size_bytes"] for bid in pick), pick)',
         "test_fewest_bytes_beat_fewest_bundles", "test_two_cheap_bundles_beat_one_dear_one_for_three_apps"),
    case("resolver-count-tie-break-dropped", 'key = (sum(bundles[bid]["size_bytes"] for bid in pick), count, pick)', 'key = (sum(bundles[bid]["size_bytes"] for bid in pick), pick)',
         "test_equal_bytes_prefer_fewer_bundles", "test_equal_bytes_prefer_fewer_bundles_for_three_apps"),
    case("resolver-id-tie-break-last-wins", [('key = (sum(bundles[bid]["size_bytes"] for bid in pick), count, pick)', 'key = (sum(bundles[bid]["size_bytes"] for bid in pick), count)'),
                                              ('if best is None or key < best:', 'if best is None or key <= best:')], None,
         "test_equal_bytes_and_bundles_prefer_the_smaller_ids", "test_equal_cost_covers_are_broken_by_the_smaller_ids"),
    case("resolver-picks-the-dearest", 'if best is None or key < best:', 'if best is None or key > best:', "test_the_cheaper_bundle_wins_even_with_a_later_id", "test_equal_bytes_prefer_fewer_bundles"),
    case("resolver-cover-not-checked", 'if all(any(bid in pick for bid in holders[name]) for name in wanted):', 'if True:',
         "test_apps_on_two_bundles_need_both", "test_both_platforms_in_one_request"),
    case("resolver-result-unsorted", 'return list(best[2])', 'return list(reversed(best[2]))', "test_result_is_sorted_by_bundle_id", "test_apps_on_two_bundles_need_both"),
    case("resolver-skips-validation", '    bundles = validate(config)["bundles"]\n    refuse(os is None', '    bundles = config["bundles"]\n    refuse(os is None',
         "test_an_invalid_configuration_is_refused_before_resolving", "test_an_invalid_bundle_table_is_refused_even_for_an_empty_request"),
    case("bytes-ignore-bundles", 'return sum(bundles[i]["size_bytes"] for i in ids)', 'return len(ids)', "test_download_bytes_sums_the_named_bundles", "test_resolve_prints_bundles_and_bytes"),
    case("bytes-unknown-bundle-allowed", 'refuse(all(i in bundles for i in ids), "UNKNOWN_BUNDLE")', 'pass', "test_download_bytes_refuses_an_unknown_bundle", "test_download_bytes_of_only_unknown_bundles_is_refused"),
]


def run(tree):
    p = subprocess.run([sys.executable, "-B", "-I", str(tree / "scripts/ci" / TEST.name), "-v"], cwd=tree / "scripts/ci", capture_output=True, text=True, timeout=300)
    out = p.stdout + p.stderr
    killers = re.findall(r"^(?:FAIL|ERROR): (test_\w+) ", out, re.M)
    return p.returncode, killers, out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    weak = 0
    with tempfile.TemporaryDirectory(prefix="propimage-config-mutants-") as tmp:
        tree = Path(tmp)
        (tree / "scripts/ci").mkdir(parents=True)
        for entry in HERE.iterdir():              # everything the tests read stays the repository's own file
            if entry.name not in ("propimage_config.py", TEST.name, "__pycache__"):
                (tree / "scripts/ci" / entry.name).symlink_to(entry)
        for top in ("ci", "docs"):
            (tree / top).symlink_to(ROOT / top)
        (tree / "scripts/bench").symlink_to(ROOT / "scripts/bench")
        shutil.copy(TEST, tree / "scripts/ci" / TEST.name)
        target = tree / "scripts/ci/propimage_config.py"
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-3000:])
            return 2
        known = set(re.findall(r"^(test_\w+) \(", out, re.M))
        unknown = sorted({t for c in CASES for t in c[2]} - known)
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, mutated, tests in CASES:
            missing = [t for t in tests if t not in known]
            target.write_text(mutated)
            try:
                code, killers, out = run(tree)
            finally:
                target.write_text(SRC)
            ok = code != 0 and not missing and all(t in killers for t in tests)
            print("%s %s: %d failing, named killers %s%s" % ("PASS" if ok else "WEAK", name, len(killers), [t for t in tests if t in killers],
                                                             (" UNKNOWN TEST " + ",".join(missing)) if missing else ""), flush=True)
            weak += not ok
    print("%d/%d mutants killed by both named tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak or unknown else 0


if __name__ == "__main__":
    sys.exit(main())
