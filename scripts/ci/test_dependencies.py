import unittest
from dependencies import checked_dependencies, dependencies, violations


class DependencyTests(unittest.TestCase):
    def test_alias_workspace_and_target_dependencies_cannot_hide_old_code(self):
        manifest = {"dependencies": {"alias": {"package": "kayfabe-old"}},
                    "target": {"cfg(unix)": {"dev-dependencies": {"shared": {"workspace": True}}}}}
        deps = list(dependencies(manifest, {"shared": {"package": "kayfabe-device"}}))
        self.assertEqual(violations("kf-trap", deps), ["kayfabe-old", "kayfabe-device"])

    def test_frozen_model_cannot_become_a_grader_dependency(self):
        self.assertEqual(violations("kayfabe-rm-ladder", ["kayfabe-doorbell"]), ["kayfabe-doorbell"])
        self.assertEqual(violations("kf-trap", ["kf-chip"]), [])

    def test_only_existing_legacy_class_table_consumer_is_allowed(self):
        self.assertEqual(violations("kayfabe-chips", ["kayfabe-doorbell"]), [])
        self.assertEqual(violations("kf-chip", ["kayfabe-doorbell"]), ["kayfabe-doorbell"])

    def test_pure_crates_cannot_import_an_os_adapter_or_external_crate(self):
        self.assertEqual(violations("kf-trap", ["libc", "kf-linux-raw"]), ["libc", "kf-linux-raw"])
        self.assertEqual(violations("kf-chan", ["kf-linux-raw"]), [])

    def test_workspace_version_string_is_supported(self):
        self.assertEqual(list(dependencies({"dependencies": {"serde": {"workspace": True}}},
                                           {"serde": "1"})), ["serde"])

    def test_named_dev_only_exceptions_are_exact(self):
        loom = {"dev-dependencies": {"loom": "=0.7.2"}}
        self.assertEqual(violations("kf-trap", checked_dependencies("kf-trap", loom, {})), [])
        # the same crate as a shipped or build dependency is still refused
        for key in ("dependencies", "build-dependencies"):
            shipped = {key: {"loom": "=0.7.2"}}
            self.assertEqual(violations("kf-trap", checked_dependencies("kf-trap", shipped, {})), ["loom"])
        # another crate, or another dev-dependency of the same crate, is still refused
        self.assertEqual(violations("kf-chan", checked_dependencies("kf-chan", loom, {})), [])
        self.assertEqual(violations("kf-core", checked_dependencies("kf-core", loom, {})), ["loom"])
        other = {"dev-dependencies": {"proptest": "1"}}
        self.assertEqual(violations("kf-trap", checked_dependencies("kf-trap", other, {})), ["proptest"])
        # a target-specific dev-dependency is covered the same way
        target = {"target": {"cfg(unix)": {"dev-dependencies": {"kf-broker": {"workspace": True}}}}}
        self.assertEqual(violations("kf-disp", checked_dependencies("kf-disp", target, {"kf-broker": {}})), [])
        self.assertEqual(violations("kf-trap", checked_dependencies("kf-trap", target, {"kf-broker": {}})), ["kf-broker"])
