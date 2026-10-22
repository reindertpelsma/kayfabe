//! Native characterization of the same optional private-ring invalid opcode.
use kf_chan::host::HostRing;
use kf_chan::rc_negative::{Arm, NativePort, inject};
use kf_host::HostRm;
use kf_linux_raw::{DevDir, PollTimeout, Poller, ReadyTokens};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn positive_fence(ring: &mut HostRing, rm: &HostRm) -> Result<(), String> {
    let sequence = ring.fence(rm)?.map_err(|_| "positive fence busy")?;
    let start = Instant::now();
    while ring.completed()? < sequence {
        if start.elapsed() > Duration::from_secs(5) {
            return Err("positive fence deadline".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if ring.rc_words().is_some() {
        return Err("positive channel has RC error".into());
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let run = std::env::args().nth(1).ok_or("explicit run id required")?;
    let arm = Arm::configured(Some(&run)).ok_or("invalid run id")?;
    let source = option_env!("KF_RC_SOURCE_REV")
        .filter(|v| v.len() == 40)
        .ok_or("build with KF_RC_SOURCE_REV")?;
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let engine = (0..10)
        .map(|n| kf_abi::submit::ENGINE_TYPE_COPY0 + n)
        .find(|&e| rm.ce_is_grce(e) == Ok(false))
        .ok_or("no asynchronous CE")?;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let mut ring = HostRing::on_engine(&rm, space, engine)?;
    let mut bystander = HostRing::on_engine(&rm, space, engine)?;
    let result = (|| {
        let event = rm.open_event_fd().map_err(|e| format!("event: {e:?}"))?;
        ring.arm_rc_event(&rm, &event)?;
        let poller = Poller::create().map_err(|e| format!("poller: {e:?}"))?;
        poller
            .watch(event.as_fd(), 1)
            .map_err(|e| format!("watch: {e:?}"))?;
        let mut ready = ReadyTokens::default();
        positive_fence(&mut ring, &rm)?;
        positive_fence(&mut bystander, &rm)?;
        if ring.rc_words().is_some()
            || poller
                .wait(&mut ready, PollTimeout::Immediate)
                .map_err(|e| format!("poll: {e:?}"))?
                != 0
        {
            return Err("fault/event before intentional submission".into());
        }
        let token = ring.channel().token;
        let unix_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        println!(
            "FAULT_MARK=translated_rc_native run={} pid={} host={token:#x} channel={:#x} source={source} word=0xc0000000 unix_ns={unix_ns}",
            arm.run(),
            std::process::id(),
            token & 0xfff
        );
        inject(&mut NativePort::new(&mut ring, &rm))?;
        let start = Instant::now();
        let mut hinted = false;
        loop {
            hinted |= poller
                .wait(&mut ready, PollTimeout::Immediate)
                .map_err(|e| format!("poll: {e:?}"))?
                != 0;
            if let Some(words) = ring.rc_words() {
                if words[2] != 32 {
                    return Err(format!(
                        "native exception {}, expected characterization Xid32",
                        words[2]
                    ));
                }
                if hinted {
                    if words[3] >> 16 != 0xffff {
                        return Err("native RC record is not published".into());
                    }
                    break;
                }
            }
            if start.elapsed() > Duration::from_secs(5) {
                return Err("private RC record/event deadline".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        positive_fence(&mut bystander, &rm)?;
        poller
            .unwatch(event.as_fd())
            .map_err(|e| format!("unwatch: {e:?}"))?;
        Ok(())
    })();
    let mut cleanup_error = None;
    for owned in [&mut ring, &mut bystander] {
        if let Err(error) = rm.free_channel(owned.channel()) {
            cleanup_error = Some(format!("channel free: {error:?}"));
        }
        match owned.release(&rm) {
            Some(message) if !message.contains("REFUSED") => {}
            message => cleanup_error = Some(format!("private ring cleanup: {message:?}")),
        }
    }
    rm.free_vaspace(space);
    result?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    println!(
        "NATIVE_RC_RESULT run={} code=32 status=0xffff event=1 bystander=1 result=PASS",
        arm.run()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("NATIVE_RC_RESULT result=FAIL error={error}");
        std::process::exit(1);
    }
}
