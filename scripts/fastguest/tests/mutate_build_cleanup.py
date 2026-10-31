"""Run named ownership mutants against inert executable builder fixtures."""

import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

root = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location(
    "build_tests", root / "scripts/fastguest/tests/test_build_cleanup.py"
)
tests = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tests)
source = (root / "scripts/fastguest/build_fast_guest.sh").read_text()
mutants = {
    "cleanup_without_ownership": (
        'if [ "$NBD_ATTACHED" = 1 ]; then',
        "if true; then",
        [
            "test_host_mode_failure_never_disconnects_another_attachment",
            "test_skip_nbd_pack_failure_never_disconnects",
        ],
    ),
    "claim_before_successful_attach": (
        "if qemu-nbd --read-only",
        "NBD_ATTACHED=1\nif qemu-nbd --read-only",
        [
            "test_failed_attach_does_not_disconnect_an_unacquired_device",
            "test_interrupted_failed_attach_does_not_claim_ownership",
        ],
    ),
    "successful_attach_not_owned": (
        "    NBD_ATTACHED=1",
        "    NBD_ATTACHED=0",
        [
            "test_owned_attachment_is_released_after_mount_failure",
            "test_owned_attachment_is_released_after_term",
        ],
    ),
    "successful_release_keeps_stale_ownership": (
        "    NBD_ATTACHED=0\nfi\ntrap -",
        "    NBD_ATTACHED=1\nfi\ntrap -",
        [
            "test_normal_completion_disconnects_exactly_once",
            "test_pack_failure_after_release_does_not_disconnect_twice",
        ],
    ),
    "failed_release_forgets_ownership": (
        "    NBD_ATTACHED=0\nfi\ntrap -",
        "fi\nNBD_ATTACHED=0\ntrap -",
        [
            "test_failed_release_is_retried_while_still_owned",
            "test_pack_failure_retries_a_failed_owned_release",
        ],
    ),
    "release_signal_not_deferred": (
        "trap 'nbd_signal=130' INT\ntrap 'nbd_signal=143' TERM\nif qemu-nbd --disconnect",
        "if qemu-nbd --disconnect",
        [
            "test_successful_release_interrupted_by_term_does_not_disconnect_twice",
            "test_successful_release_interrupted_by_int_does_not_disconnect_twice",
        ],
    ),
    "attach_signal_not_deferred": (
        "trap 'nbd_signal=130' INT\ntrap 'nbd_signal=143' TERM\nif qemu-nbd --read-only",
        "if qemu-nbd --read-only",
        [
            "test_successful_attach_interrupted_by_term_is_released",
            "test_successful_attach_interrupted_by_int_is_released",
        ],
    ),
}
results = {}
with tempfile.TemporaryDirectory() as folder:
    for name, (old, new, selected) in mutants.items():
        assert source.count(old) == 1, (name, source.count(old))
        tests.BUILDER = Path(folder) / (name + ".sh")
        tests.BUILDER.write_text(source.replace(old, new))
        suite = unittest.TestSuite(tests.BuildCleanup(test) for test in selected)
        result = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
        killed = [test._testMethodName for test, _ in result.failures + result.errors]
        results[name] = {"tests": result.testsRun, "killers": killed}
        print(name, len(killed), killed, flush=True)
        assert len(killed) >= 2, name
print(json.dumps(results, indent=2))
