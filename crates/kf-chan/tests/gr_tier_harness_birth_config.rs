//! Native harness birth configs must compile and reflect each ring's host facts.
//! FALSIFIER: the actual graphics literal seeds CE, or the copy literal ignores
//! its own allocated class. These probes execute no GPU or host RM operation.
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const HARNESS: &str = include_str!("../../kf-harness/src/bin/kf-gr-tier.rs");
const DECODER: &str = include_str!("../src/tmode.rs");
static SERIAL: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(0);

struct ProbeDir(PathBuf);
impl Drop for ProbeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config_expression(receiver: &str) -> &str {
    let marker = format!("{receiver}.set_gr(");
    let (_, rest) = HARNESS
        .split_once(&marker)
        .expect("real harness config call");
    let (expr, _) = rest.split_once(");").expect("config expression end");
    expr
}

fn probe(receiver: &str, host_class: u32, expected_ce: u32, graphics: bool) {
    // Serialize child compilers even when the Rust test runner uses many threads.
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_, fields) = DECODER
        .split_once("pub struct GrConfig {")
        .expect("real config type");
    let (fields, _) = fields.split_once("\n}").expect("config fields end");
    let dir = ProbeDir(std::env::temp_dir().join(format!(
        "kf-f1-gr-config-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    // Copy the actual type's fields and the actual native initializer verbatim.
    // The stand-in only supplies the host fact, with a different class on each
    // host object so accidentally reading the GR ring's class also fails.
    let source = format!(
        r#"
pub struct GrConfig {{ {fields} }}
struct HostFact(u32);
impl HostFact {{ fn ce_class(&self) -> u32 {{ self.0 }} }}
fn main() {{
    let host = HostFact(0xdead);
    let host2 = HostFact({host_class});
    let cfg = {expr};
    assert_eq!(cfg.ce, {expected_ce}, "class must follow this ring's configuration");
    assert_eq!(cfg.tier, {graphics});
    assert!(cfg.inert_sw_subch);
    assert!(!cfg.deferred_api);
    assert_eq!(cfg.video, 0);
}}
"#,
        expr = config_expression(receiver)
    );
    let src = dir.0.join("probe.rs");
    let bin = dir.0.join("probe");
    std::fs::write(&src, source).unwrap();
    let compiled = Command::new("rustc")
        .args(["--edition=2024", "--crate-name", "gr_config_probe"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("Rust toolchain used by Cargo is available");
    assert!(
        compiled.status.success(),
        "actual {receiver} initializer must compile with current GrConfig: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = Command::new(&bin).output().unwrap();
    assert!(
        ran.status.success(),
        "actual {receiver} initializer, host class {host_class:#x}: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
}

#[test]
fn graphics_c5_host_keeps_its_explicit_grce_binding() {
    probe("ring", 0xc5b5, 0, true);
}
#[test]
fn graphics_c7_host_keeps_its_explicit_grce_binding() {
    probe("ring", 0xc7b5, 0, true);
}
#[test]
fn graphics_c8_host_keeps_its_explicit_grce_binding() {
    probe("ring", 0xc8b5, 0, true);
}
#[test]
fn dedicated_copy_c5_uses_its_allocated_host_class() {
    probe("ring2", 0xc5b5, 0xc5b5, false);
}
#[test]
fn dedicated_copy_c7_uses_its_allocated_host_class() {
    probe("ring2", 0xc7b5, 0xc7b5, false);
}
#[test]
fn dedicated_copy_c8_uses_its_allocated_host_class() {
    probe("ring2", 0xc8b5, 0xc8b5, false);
}
