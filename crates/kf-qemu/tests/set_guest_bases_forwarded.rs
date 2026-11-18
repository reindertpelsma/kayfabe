//! ★ 2026-11-18 (C2, `docs/design/V3_SPARSE_RESIDENCY_RCA.md` §8.11): every production `MapTarget` that forwards
//! `begin_refresh` (a wrapping or delegating target) also forwards `set_guest_bases`. `[measured box
//! kf-openguest-180716, tree f5dbe55e]` `GpuMirror` and `Target` did not: the trait default refused every SPARSE run
//! ("this target has no exact tiling") on a tiled space, and the Vulkan sparse-residency cases still hung with
//! `exact-sparse=on`. The GPU-free model (`SimMirror`) forwarded it, so no model test could see the gap; this reads
//! the production sources instead.

fn impls(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("MapTarget for ") {
        let after = &rest[i + "MapTarget for ".len()..];
        let name: String = after.chars().take_while(|c| !c.is_whitespace() && *c != '{').collect();
        // The impl body: from its opening brace to the matching close.
        let Some(open) = after.find('{') else { break };
        let mut depth = 0usize;
        let mut end = open;
        for (k, ch) in after[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push((name, after[open..=end].to_string()));
        rest = &after[end..];
    }
    out
}

#[test]
fn every_delegating_map_target_forwards_set_guest_bases() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for file in ["crates/kf-qemu/src/mem.rs", "crates/kf-harness/src/publish.rs"] {
        let src = std::fs::read_to_string(root.join(file)).expect(file);
        for (name, body) in impls(&src) {
            if body.contains("fn begin_refresh") {
                seen += 1;
                assert!(
                    body.contains("fn set_guest_bases"),
                    "{file}: `impl MapTarget for {name}` forwards begin_refresh but not set_guest_bases"
                );
            }
        }
    }
    assert!(seen >= 3, "found {seen} delegating targets (GpuMirror, Target, Recorded expected)");
}
