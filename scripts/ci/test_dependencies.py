import unittest
from dependencies import dependencies, kf3_violations, violations


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


def _meta(root, packages, edges, features=None, members=None, builds=()):
    """A minimal `cargo metadata` document: packages are (name, rel-dir or None for registry)."""
    features = features or {}
    pkgs, nodes = [], []
    for name, rel in packages:
        pid = f"{name}-id"
        manifest = f"{root}/{rel}/Cargo.toml" if rel else f"/registry/{name}/Cargo.toml"
        targets = [{"kind": ["lib"], "name": name, "src_path": manifest.replace("Cargo.toml", "src/lib.rs")}]
        if name in builds:
            targets.append({"kind": ["custom-build"], "name": "build-script-build",
                            "src_path": manifest.replace("Cargo.toml", "build.rs")})
        pkgs.append({"id": pid, "name": name, "manifest_path": manifest,
                     "source": None if rel else "registry+https://github.com/rust-lang/crates.io-index",
                     "targets": targets})
        deps = [{"pkg": f"{b}-id", "dep_kinds": [{"kind": k, "target": None}]} for a, b, k in edges if a == name]
        nodes.append({"id": pid, "deps": deps, "features": features.get(name, [])})
    members = members if members is not None else [f"{n}-id" for n, rel in packages if rel]
    return {"packages": pkgs, "workspace_members": members, "resolve": {"nodes": nodes}}


KF3 = {"root": "kf-qemu", "external": {"libc": ["default", "std"]},
       "member_features": {"kf-oprom": ["alloc", "default"]}, "custom_build": ["kf-gop-image", "libc"]}


class Kf3ClosureTests(unittest.TestCase):
    """M6 (V3_SEC_PERIMETER.md §1.5): each rule has a fixture that must fail it."""

    def base(self, extra_pkgs=(), extra_edges=(), features=None, builds=("kf-gop-image", "libc"), members=None):
        pk = [("kf-qemu", "crates/kf-qemu"), ("kf-linux-raw", "crates/kf-linux-raw"),
              ("kf-gop-image", "crates/kf-gop-image"), ("kf-oprom", "crates/kf-oprom"),
              ("libc", None), *extra_pkgs]
        ed = [("kf-qemu", "kf-linux-raw", None), ("kf-linux-raw", "libc", None),
              ("kf-qemu", "kf-gop-image", "build"), ("kf-qemu", "kf-oprom", None), *extra_edges]
        f = {"libc": ["default", "std"], "kf-oprom": ["alloc", "default"]}
        f.update(features or {})
        return _meta("/r", pk, ed, f, members, builds)

    def test_the_reviewed_closure_passes(self):
        self.assertEqual(kf3_violations(self.base(), KF3, "/r"), [])

    def test_a_new_external_crate_in_the_closure_fails(self):
        m = self.base(extra_pkgs=[("memchr", None)], extra_edges=[("kf-linux-raw", "memchr", None)])
        self.assertTrue(any("memchr" in v for v in kf3_violations(m, KF3, "/r")))

    def test_a_dev_only_external_crate_does_not_enter_the_closure(self):
        m = self.base(extra_pkgs=[("trybuild", None)], extra_edges=[("kf-linux-raw", "trybuild", "dev")])
        self.assertEqual(kf3_violations(m, KF3, "/r"), [])

    def test_a_build_dependency_is_part_of_the_closure(self):
        m = self.base(extra_pkgs=[("cc", None)], extra_edges=[("kf-gop-image", "cc", "build")])
        self.assertTrue(any("cc" in v for v in kf3_violations(m, KF3, "/r")))

    def test_a_member_outside_crates_kf_fails(self):
        m = self.base(extra_pkgs=[("kf-odd", "tools/kf-odd")], extra_edges=[("kf-qemu", "kf-odd", None)])
        self.assertTrue(any("kf-odd" in v for v in kf3_violations(m, KF3, "/r")))

    def test_a_non_member_path_package_fails_even_when_named_kf(self):
        m = self.base(extra_pkgs=[("kf-x", "crates/kf-x")], extra_edges=[("kf-qemu", "kf-x", None)],
                      members=["kf-qemu-id", "kf-linux-raw-id", "kf-gop-image-id", "kf-oprom-id"])
        self.assertTrue(any("kf-x" in v for v in kf3_violations(m, KF3, "/r")))

    def test_a_new_build_script_fails(self):
        m = self.base(builds=("kf-gop-image", "libc", "kf-linux-raw"))
        self.assertTrue(any("build scripts" in v for v in kf3_violations(m, KF3, "/r")))

    def test_force_host_page_size_cannot_be_switched_on_for_kf3(self):
        m = self.base(features={"kf-linux-raw": ["force-host-page-size"]})
        self.assertTrue(any("kf-linux-raw enabled features" in v for v in kf3_violations(m, KF3, "/r")))

    def test_an_external_feature_change_fails(self):
        m = self.base(features={"libc": ["default", "extra_traits", "std"]})
        self.assertTrue(any("libc enabled features" in v for v in kf3_violations(m, KF3, "/r")))
