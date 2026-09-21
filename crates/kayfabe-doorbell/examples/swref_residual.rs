use kayfabe_doorbell::swref;
fn main() {
    let root = "/workspace/nvidia-gpu-passthrough/research_clones/ogkm/src/common/inc/swref";
    let mut stack = vec![std::path::PathBuf::from(root)];
    let mut shown = 0;
    let mut total = 0;
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { stack.push(path); continue; }
            if path.extension().and_then(|s| s.to_str()) != Some("h") { continue; }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines() {
                let t = line.trim();
                if !t.starts_with("#define ") { continue }
                // the independent predicate: a 5-char code in the comment
                let Some((_h, tail)) = t.split_once("/*") else { continue };
                let Some(code) = tail.trim_start().split_whitespace().next() else { continue };
                if code.len() != 5 || !code.chars().all(|c| c == '-' || c.is_ascii_alphabetic()) { continue }
                total += 1;
                if swref::parse_line(line).is_none() {
                    shown += 1;
                    if shown <= 15 { println!("UNPARSED: {}", t); }
                }
            }
        }
    }
    println!("independent_total={total} unparsed={shown}");
}
