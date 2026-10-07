//! Native constructor/context-lifetime oracle, not a codec execution claim.
use kf_chan::host::HostRing;
use kf_host::HostRm;
use kf_linux_raw::DevDir;

fn main() {
    match run() {
        Ok(()) => println!("VIDEO_CONTEXT_VERDICT=PASS"),
        Err(e) => {
            eprintln!("VIDEO_CONTEXT_VERDICT=FAIL {e}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let nvenc = std::env::var("KF_NVENC_CONTEXT_INDEX").ok();
    let ofa = std::env::var("KF_OFA_CONTEXT_INDEX").ok();
    let engine = match (nvenc, ofa) {
        (Some(s), None) => s
            .parse::<u32>()
            .ok()
            .and_then(kf_abi::submit::engine_type_nvenc)
            .ok_or("KF_NVENC_CONTEXT_INDEX must select NVENC0..3")?,
        (None, Some(s)) => s
            .parse::<u32>()
            .ok()
            .and_then(kf_abi::submit::engine_type_ofa)
            .ok_or("KF_OFA_CONTEXT_INDEX must select OFA0..1")?,
        (None, None) => kf_abi::submit::engine_type_nvdec(0).ok_or("NVDEC0 engine")?,
        _ => return Err("select one video engine kind".into()),
    };
    let mut ring = HostRing::on_engine(&rm, space, engine)?;
    println!(
        "VIDEO_CONTEXT driver={} arch={:x?} context={:x?} channel={:x?}",
        rm.driver_version(),
        rm.arch_info(),
        ring.video_context(),
        ring.channel()
    );
    let result = (|| {
        if !ring.owns_context(engine) || ring.owns_context(kf_abi::submit::ENGINE_TYPE_GRAPHICS) {
            return Err("context ownership mismatch".into());
        }
        // A real codec selector must refuse before writing any PB/GP entry.
        let selector = [
            kf_abi::submit::method_header_inc(0, kf_abi::submit::SET_OBJECT, 1)
                .ok_or("selector header")?,
            ring.video_context().ok_or("no context")?.1,
        ];
        if ring.push(&selector).is_ok() {
            return Err("codec submission was admitted".into());
        }
        println!("VIDEO_CODEC_NEGATIVE=REFUSED");
        let seq = ring.fence(&rm)?.map_err(|_| "fence busy")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let done = ring.completed()?;
            if done == seq {
                println!("VIDEO_FENCE seq={seq} completed={done}");
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("fence timeout seq={seq} completed={done}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(())
    })();
    let freed = rm
        .free_channel(ring.channel())
        .map_err(|e| format!("free channel: {e:?}"));
    println!("VIDEO_RELEASE {:?}", ring.release(&rm));
    rm.free_vaspace(space);
    result.and(freed)
}
