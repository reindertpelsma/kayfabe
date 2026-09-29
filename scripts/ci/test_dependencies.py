import unittest
from dependencies import dependencies, violations


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
