//! Capability gate for the complete CpuIntr apply/read/shadow/reset implementation
//! and the research replay module. Compile the actual source under no_std, without
//! alloc or OS handles. Only the unrelated String diagnostic snapshot is omitted.
//! This catches std locks, allocation and syscalls, not arbitrary hand-written
//! atomic spin locks. It is deliberately not called a linearizability proof.
use std::process::Command;

fn source() -> String {
    let original =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/cpuintr.rs")).unwrap();
    let mut source = original.split("#[cfg(test)]").next().unwrap().to_owned();
    let begin = source.find("    pub fn snapshot(&self)").unwrap();
    let open = begin + source[begin..].find('{').unwrap();
    let mut depth = 1;
    let mut end = open + 1;
    // The current diagnostic body has balanced braces, including its format strings.
    for (i, b) in source[open + 1..].bytes().enumerate() {
        if b == b'{' {
            depth += 1;
        }
        if b == b'}' {
            depth -= 1;
        }
        if depth == 0 {
            end = open + 1 + i + 1;
            break;
        }
    }
    source.replace_range(begin..end, "");
    source
        .lines()
        .filter(|l| !l.trim_start().starts_with("///"))
        .map(|l| l.replace("//!", "//") + "\n")
        .collect()
}

fn compile(tag: &str, replay: bool) {
    let temp = std::env::temp_dir().join(format!("r1-no-block-{}-{tag}", std::process::id()));
    std::fs::create_dir(&temp).unwrap();
    let mut code = String::from(
        "#![no_std]\nextern crate self as kf_chip;\n#[derive(Clone,Copy)] pub enum Family {Turing,Ampere,Ada,Hopper,Blackwell}\nmod mmuinval { pub const USERMODE_ABOVE_PRIV: u64 = 0x30000; }\nmod cpuintr {\n",
    );
    code += &source();
    code += "\n}\n";
    if replay {
        let replay = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/cpuintr_replay.rs"
        ))
        .unwrap();
        code += "mod cpuintr_replay {\n";
        code += &replay.replace("//!", "//");
        code += "\n}\n";
        let audit =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/cpuintr_audit.rs"))
                .unwrap();
        code += "mod cpuintr_audit {\n";
        code += &audit.replace("//!", "//");
        code += "\n}\n";
    }
    let input = temp.join("gate.rs");
    std::fs::write(&input, code).unwrap();
    let output = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-type=rlib",
            "--emit=metadata",
            "-Adead_code",
        ])
        .arg(&input)
        .arg("-o")
        .arg(temp.join("gate.rmeta"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&temp).unwrap();
    assert!(
        output.status.success(),
        "{tag}: apply acquired a capability unavailable under no_std without alloc:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cpuintr_apply_has_no_std_lock_allocator_or_os_capability() {
    compile("apply", false);
}

#[test]
fn replay_and_its_transitive_apply_have_no_blocking_capability() {
    compile("replay", true);
}
