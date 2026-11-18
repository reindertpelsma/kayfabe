//! ★ 2026-11-18 — **bare-metal probe for the Blackwell private-ring marker** (`docs/design/V3_BLACKWELL_USER_RING_ROUTE.md`
//! §2.7). Native host client only: no VM, no guest memory. Each attempt is ONE child process with a fresh VA space, a
//! fresh USER copy channel, owned memory and a read-only command mapping. The only GPU-writable mapping is one owned
//! 64 KiB buffer whose first word the stream releases into.
//!
//! The parent runs `N` attempts of each variant (A, B, C; `kf_harness::semprobe`):
//! - it records the exact words and methods before each attempt;
//! - it snapshots host dmesg, starts the child, and KILLS it on a hard watchdog;
//! - it reads the dmesg delta for `NVRM: Xid` lines and classifies the attempt;
//! - it STOPS at the first unexpected outcome (fail closed), keeping the evidence on stdout.
//!
//! Usage: `kf-semmarker-probe [ATTEMPTS]` (default 3; GPU by `KF_GATE_GPU`, as the gates).
//! Internal: `kf-semmarker-probe child <A|B|C>`.
use kf_abi::submit::{ENGINE_TYPE_COPY0, USERD_GP_PUT, USERD_REFERENCE_C56F, gp_entry};
use kf_harness::semprobe::{self, Attempt, Variant, Verdict};
use kf_host::{HostRm, MapNode, MapPerm, VaSpace, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::io::Read;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const BYTES: u64 = 64 << 10;
const GP: u64 = 0x1000;
const PB: u64 = 0x4000;
/// How long the child polls the owned word.
const POLL: Duration = Duration::from_secs(3);
/// The parent's hard watchdog per child.
const WATCHDOG: Duration = Duration::from_secs(20);
/// After a kill, how long the parent waits for the child to be reaped before giving up on it.
const REAP: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("child") {
        let Some(v) = args.get(2).and_then(|s| Variant::parse(s)) else {
            eprintln!("usage: kf-semmarker-probe child A|B|C");
            return ExitCode::from(2);
        };
        return match child(v) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                println!("SEMPROBE_CHILD_ERROR variant={} error={e}", v.letter());
                ExitCode::from(3)
            }
        };
    }
    let attempts: u32 = match args.get(1).map(|s| s.parse()) {
        None => 3,
        Some(Ok(n)) if (1..=10).contains(&n) => n,
        _ => {
            eprintln!("usage: kf-semmarker-probe [ATTEMPTS 1..=10]");
            return ExitCode::from(2);
        }
    };
    match parent(attempts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            println!("SEMPROBE_STOP {e}");
            ExitCode::from(1)
        }
    }
}

fn dmesg_lines() -> Result<Vec<String>, String> {
    let out = Command::new("dmesg")
        .output()
        .map_err(|e| format!("dmesg: {e}"))?;
    if !out.status.success() {
        return Err(format!("dmesg exited {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

/// The lines of `after` that are not in the tail of `before` (by position of `before`'s last line).
fn dmesg_delta(before: &[String], after: &[String]) -> Vec<String> {
    let start = before
        .last()
        .and_then(|last| after.iter().rposition(|l| l == last))
        .map_or(0, |p| p + 1);
    after[start..].to_vec()
}

fn parent(attempts: u32) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    println!(
        "SEMPROBE_START attempts={attempts} watchdog_s={} poll_s={} gpu_minor={}",
        WATCHDOG.as_secs(),
        POLL.as_secs(),
        kf_harness::gate_gpu().0
    );
    for v in Variant::ALL {
        for n in 1..=attempts {
            // The exact words, recorded before the GPU sees them (the VA is the child's; the words are printed
            // again by the child with its real VA).
            let shape = semprobe::methods(&semprobe::words(v, 0x1000).map_err(str::to_owned)?);
            println!(
                "SEMPROBE_ATTEMPT variant={} attempt={n} methods={shape:x?}",
                v.letter()
            );
            let before = dmesg_lines()?;
            let started = Instant::now();
            let mut c = Command::new(&me)
                .args(["child", &v.letter().to_string()])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| format!("spawn: {e}"))?;
            let mut exited = false;
            while started.elapsed() < WATCHDOG {
                if c.try_wait().map_err(|e| format!("wait: {e}"))?.is_some() {
                    exited = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let mut status = None;
            if exited {
                status = c.try_wait().ok().flatten();
            } else {
                let _ = c.kill();
                let killed = Instant::now();
                while killed.elapsed() < REAP {
                    if let Ok(Some(s)) = c.try_wait() {
                        status = Some(s);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                println!(
                    "SEMPROBE_WATCHDOG variant={} attempt={n} killed=true reaped={}",
                    v.letter(),
                    status.is_some()
                );
            }
            let mut out = String::new();
            if status.is_some()
                && let Some(mut so) = c.stdout.take()
            {
                let _ = so.read_to_string(&mut out);
            }
            for l in out.lines() {
                println!("  child: {l}");
            }
            // Give the driver a moment to print an Xid raised at teardown.
            std::thread::sleep(Duration::from_millis(500));
            let delta = dmesg_delta(&before, &dmesg_lines()?);
            for l in delta.iter().filter(|l| l.contains("NVRM")) {
                println!("  dmesg: {l}");
            }
            let word = out.lines().find_map(|l| {
                l.strip_prefix("SEMPROBE_WORD value=")
                    .and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok())
            });
            let a = Attempt {
                exited: exited && status.is_some_and(|s| s.success()),
                word,
                xids: delta
                    .iter()
                    .filter_map(|l| semprobe::parse_xid(l))
                    .collect(),
            };
            let verdict = semprobe::verdict(v, &a);
            println!(
                "SEMPROBE_RESULT variant={} attempt={n} exited={} status={status:?} word={:x?} expected={:#x} xids={:x?} elapsed_ms={} verdict={verdict:?}",
                v.letter(),
                a.exited,
                a.word,
                v.expected_word(),
                a.xids,
                started.elapsed().as_millis()
            );
            if verdict == Verdict::Unexpected {
                return Err(format!(
                    "variant={} attempt={n}: unexpected outcome, run stopped (fail closed)",
                    v.letter()
                ));
            }
        }
    }
    println!("SEMPROBE_DONE");
    Ok(())
}

struct Buffer {
    memory: u32,
    _node: kf_linux_raw::CharDevice,
    cookie: u64,
    cpu: VolatileRegion,
    va: Option<u64>,
}

impl Buffer {
    fn new(rm: &HostRm, space: VaSpace, perm: Option<MapPerm>) -> Result<Self, String> {
        let memory = rm
            .alloc_device_local(BYTES)
            .map_err(|e| format!("memory: {e:?}"))?;
        let va = perm
            .map(|p| rm.map_window_perm(space, memory, BYTES, false, p))
            .transpose()
            .map_err(|e| format!("map: {e:?}"))?;
        if va.is_some_and(|v| v.checked_add(BYTES).is_none_or(|e| e > 1 << 40)) {
            return Err("mapping beyond 40 bits".into());
        }
        let (node, cookie) = rm
            .arm_cpu_view(MapNode::Gpu, memory, 0, BYTES, ViewAccess::ReadWrite)
            .map_err(|e| format!("view: {e:?}"))?;
        let cpu = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            BYTES,
            CachePolicy::Uncached,
            HostPageSize::query(),
        )
        .map_err(|e| format!("mmap: {e:?}"))?;
        let b = Self {
            memory,
            _node: node,
            cookie,
            cpu,
            va,
        };
        for off in (0..BYTES).step_by(4) {
            b.store(off, 0)?;
        }
        Ok(b)
    }

    fn store(&self, off: u64, value: u32) -> Result<(), String> {
        self.cpu
            .store_u32(HostOffset::new(off), value)
            .map_err(|e| format!("store: {e:?}"))
    }

    fn load(&self, off: u64) -> Result<u32, String> {
        self.cpu
            .load_u32(HostOffset::new(off))
            .map_err(|e| format!("load: {e:?}"))
    }

    fn close(self, rm: &HostRm, space: VaSpace) -> Result<(), String> {
        drop(self.cpu);
        let view = rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: self.memory,
            p_linear_address: self.cookie,
        });
        let unmap = self.va.map(|va| rm.unmap(space, va, false)).transpose();
        let memory = rm.free(self.memory);
        view.map_err(|e| format!("release view: {e:?}"))
            .and(unmap.map(|_| ()).map_err(|e| format!("unmap: {e:?}")))
            .and(memory.map_err(|e| format!("free memory: {e:?}")))
    }
}

fn child(v: Variant) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let (architecture, implementation, _) = rm.arch_info();
    let channel_class =
        kf_chip::choose_host_classes(architecture, implementation, rm.supported_class_ids())?
            .gpfifo_channel()
            .channel_id()
            .0;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let ring = Buffer::new(
        &rm,
        space,
        Some(MapPerm {
            read_only: true,
            ..MapPerm::READ_WRITE
        }),
    )?;
    let word = Buffer::new(&rm, space, Some(MapPerm::READ_WRITE))?;
    let userd = Buffer::new(&rm, space, None)?;
    let ring_va = ring.va.ok_or("ring VA absent")?;
    let word_va = word.va.ok_or("word VA absent")?;
    let chan = rm
        .birth_channel(
            space,
            ENGINE_TYPE_COPY0,
            kf_host::RingSpec {
                gp_fifo_va: ring_va + GP,
                gp_fifo_entries: 32,
                userd_memory: userd.memory,
                userd_offset: 0,
                err_notifier: 0,
            },
        )
        .map_err(|e| format!("birth: {e:?}"))?;
    let result = (|| {
        chan.assert_user().map_err(str::to_string)?;
        rm.alloc_ce_object(chan, ENGINE_TYPE_COPY0)
            .map_err(|e| format!("CE: {e:?}"))?;
        let words = semprobe::words(v, word_va).map_err(str::to_owned)?;
        println!(
            "SEMPROBE_SETUP variant={} arch={architecture:#x} impl={implementation:#x} driver={} channel_class={:#x} user=true ring_gpu_read_only=true word_va={word_va:#x}",
            v.letter(),
            rm.driver_version(),
            channel_class
        );
        println!(
            "SEMPROBE_WORDS variant={} words={words:08x?} methods={:x?}",
            v.letter(),
            semprobe::methods(&words)
        );
        for (i, &w) in words.iter().enumerate() {
            ring.store(PB + 4 * i as u64, w)?;
        }
        let gp = gp_entry(ring_va + PB, 4 * words.len() as u64).ok_or("GP encoding")?;
        ring.store(GP, gp as u32)?;
        ring.store(GP + 4, (gp >> 32) as u32)?;
        rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
        kf_linux_raw::release_fence();
        userd.store(USERD_GP_PUT, 1)?;
        kf_linux_raw::release_fence();
        rm.doorbell(chan.token)
            .map_err(|e| format!("doorbell: {e:?}"))?;
        let start = Instant::now();
        let mut value = word.load(0)?;
        while value != v.expected_word() && start.elapsed() < POLL {
            std::thread::sleep(Duration::from_millis(1));
            value = word.load(0)?;
        }
        println!(
            "SEMPROBE_WORD value={value:#x}\nSEMPROBE_USERD reference_0x48={:#x} elapsed_us={}",
            userd.load(USERD_REFERENCE_C56F)?,
            start.elapsed().as_micros()
        );
        Ok(())
    })();
    let freed = rm
        .free_channel(chan)
        .map_err(|e| format!("channel free: {e:?}"));
    let mut cleanup = freed;
    for b in [ring, word, userd] {
        if let Err(e) = b.close(&rm, space) {
            cleanup = cleanup.and(Err(e));
        }
    }
    rm.free_vaspace(space);
    println!("SEMPROBE_CLEANUP result={cleanup:?}");
    result.and(cleanup)
}

#[cfg(test)]
mod tests {
    use super::dmesg_delta;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn the_delta_is_what_came_after_the_last_line_seen() {
        let before = s(&["a", "b"]);
        assert_eq!(
            dmesg_delta(&before, &s(&["a", "b", "x", "y"])),
            s(&["x", "y"])
        );
        assert_eq!(
            dmesg_delta(&before, &s(&["b", "x"])),
            s(&["x"]),
            "the ring rotated"
        );
        assert_eq!(
            dmesg_delta(&before, &s(&["q", "x"])),
            s(&["q", "x"]),
            "rotated past it: all"
        );
        assert_eq!(dmesg_delta(&[], &s(&["x"])), s(&["x"]));
        assert_eq!(dmesg_delta(&before, &s(&["a", "b"])), Vec::<String>::new());
    }
}
