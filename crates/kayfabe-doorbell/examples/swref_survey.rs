use kayfabe_doorbell::swref;
fn main() {
    let root = "/workspace/nvidia-gpu-passthrough/research_clones/ogkm/src/common/inc/swref";
    let mut files = 0usize; let mut n = 0usize; let mut r = 0usize; let mut w = 0usize;
    let mut off = 0usize; let mut br = 0usize; let mut ap = 0usize; let mut sb = 0usize;
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() { stack.push(path); continue; }
            if path.extension().and_then(|s| s.to_str()) != Some("h") { continue; }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            files += 1;
            for d in swref::parse_header(&text) {
                n += 1;
                if d.readable_hint { r += 1 }
                if d.writable_hint { w += 1 }
                match d.value {
                    swref::Value::Offset(_) => off += 1,
                    swref::Value::BitRange { .. } => br += 1,
                    swref::Value::Aperture { .. } => ap += 1,
                    swref::Value::StructBits { .. } => sb += 1,
                }
            }
        }
    }
    println!("files={files} parsed={n} readable={r} writable={w} offsets={off} bitranges={br} apertures={ap} structbits={sb}");
}
