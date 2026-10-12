//! Native P0 probe for the Windows user-ring design, not a VM submission path.
//! Public NVOS46 permissions; USER channel; USERD has no GPU VA mapping.
//! The writable output is a probe fixture standing in for guest-owned output,
//! not permission to put a writable host fence in a Passthrough space.
use kf_abi::submit::{USERD_GP_GET, USERD_GP_PUT, fifo, gp_entry, method_header_inc};
use kf_host::{HostRm, MapNode, MapPerm, VaSpace, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, DevDir, HostOffset, HostPageSize, VolatileRegion};
use std::time::{Duration, Instant};

const BYTES: u64 = 64 << 10;
const GP: u64 = 0x1000;
const PB: u64 = 0x4000;
const SENTINEL: u64 = 0x8000;
const INITIAL: u32 = 0x3141_5926;
const ATTEMPT: u32 = 0x2718_2818;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ReadWrite,
    ReadOnly,
    WriteAttempt,
}

impl Mode {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "read-write" => Ok(Self::ReadWrite),
            "read-only" => Ok(Self::ReadOnly),
            "write-attempt" => Ok(Self::WriteAttempt),
            _ => Err("usage: kf-user-ring-probe read-write|read-only|write-attempt".into()),
        }
    }

    fn perm(self) -> MapPerm {
        MapPerm {
            read_only: self != Self::ReadWrite,
            ..MapPerm::READ_WRITE
        }
    }
}

struct Buffer {
    memory: u32,
    // Kept open for the CPU view's lifetime.
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
        // Early setup errors leave objects to the native RM client's teardown;
        // no channel has been scheduled yet.
        let va = perm
            .map(|p| rm.map_window_perm(space, memory, BYTES, false, p))
            .transpose()
            .map_err(|e| format!("map: {e:?}"))?;
        if va.is_some_and(|v| v.checked_add(BYTES).is_none_or(|e| e > 1 << 40)) {
            return Err("probe mapping exceeds public40-bit GP/semaphore address width".into());
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

    // Called only AFTER successful channel free. Try every cleanup operation.
    fn close(self, rm: &HostRm, space: VaSpace) -> Result<(), String> {
        drop(self.cpu);
        let view = rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: self.memory,
            p_linear_address: self.cookie,
        });
        let unmap = self.va.map(|va| rm.unmap(space, va, false)).transpose();
        let memory = rm.free(self.memory);
        println!("USER_RING_BUFFER_CLEANUP view={view:?} unmap={unmap:?} memory={memory:?}");
        view.map_err(|e| format!("release view: {e:?}"))
            .and(unmap.map(|_| ()).map_err(|e| format!("unmap: {e:?}")))
            .and(memory.map_err(|e| format!("free memory: {e:?}")))
    }
}

fn release(words: &mut Vec<u32>, va: u64, value: u32) -> Result<(), String> {
    words.push(method_header_inc(0, fifo::SEM_ADDR_LO, 4).ok_or("semaphore header")?);
    words.extend_from_slice(&[va as u32, (va >> 32) as u32, value, 0]);
    words.push(method_header_inc(0, fifo::SEM_EXECUTE, 1).ok_or("execute header")?);
    words.push(fifo::SEM_EXECUTE_RELEASE_32BIT | fifo::SEM_EXECUTE_RELEASE_WFI_EN);
    Ok(())
}

fn commands(mode: Mode, ring_va: u64, output_va: u64) -> Result<Vec<u32>, String> {
    let mut words = Vec::new();
    if mode == Mode::WriteAttempt {
        release(&mut words, ring_va + SENTINEL, ATTEMPT)?;
    }
    release(&mut words, output_va, 1)?;
    Ok(words)
}

fn run(mode: Mode) -> Result<(), String> {
    let dev = DevDir::open(c"/dev").map_err(|e| format!("dev: {e:?}"))?;
    let rm = HostRm::open(&dev, kf_harness::gate_gpu(), &kf_chip::choose_host_classes)
        .map_err(|e| e.to_string())?;
    let space = rm
        .alloc_vaspace_bare()
        .map_err(|e| format!("space: {e:?}"))?;
    let ring = Buffer::new(&rm, space, Some(mode.perm()))?;
    let output = Buffer::new(&rm, space, Some(MapPerm::READ_WRITE))?;
    let userd = Buffer::new(&rm, space, None)?;
    let ring_va = ring.va.ok_or("ring VA absent")?;
    let output_va = output.va.ok_or("output VA absent")?;
    ring.store(SENTINEL, INITIAL)?;
    let chan = rm
        .birth_channel(
            space,
            kf_abi::submit::ENGINE_TYPE_GRAPHICS,
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
        println!(
            "USER_RING_SETUP driver={} mode={mode:?} channel={chan:x?} ring_va={ring_va:#x} output_va={output_va:#x} perm={:?} userd_gpu_mapped=false",
            rm.driver_version(),
            mode.perm()
        );
        let words = commands(mode, ring_va, output_va)?;
        for (i, &word) in words.iter().enumerate() {
            ring.store(PB + 4 * i as u64, word)?;
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
        let finished = loop {
            if output.load(0)? == 1 {
                break true;
            }
            if start.elapsed() > Duration::from_secs(3) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let sentinel = ring.load(SENTINEL)?;
        println!(
            "USER_RING_OBSERVATION finished={finished} fence={} sentinel={sentinel:#x} initial={INITIAL:#x} attempted={ATTEMPT:#x} GP_GET={} GP_PUT={} elapsed_us={}",
            output.load(0)?,
            userd.load(USERD_GP_GET)?,
            userd.load(USERD_GP_PUT)?,
            start.elapsed().as_micros()
        );
        // A negative case has no internal PASS: its journal + unchanged bytes
        // must be assessed externally. A deadline is collection, not completion.
        if mode == Mode::WriteAttempt {
            return Err(
                "write-attempt collected; journal attribution required, no application verdict"
                    .into(),
            );
        }
        if !finished || sentinel != INITIAL {
            return Err("GPU tail missing or private bytes changed".into());
        }
        Ok(())
    })();
    // Never revoke memory after a failed channel free. Native RM client teardown
    // owns that case, instead of treating the bench deadline as quiescence.
    rm.free_channel(chan)
        .map_err(|e| format!("channel free: {e:?}"))?;
    let mut cleanup = Ok(());
    for b in [ring, output, userd] {
        if let Err(e) = b.close(&rm, space) {
            cleanup = Err(e);
        }
    }
    rm.free_vaspace(space);
    println!("USER_RING_CLEANUP result={cleanup:?}");
    result.and(cleanup)
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = if args.len() == 1 {
        Mode::parse(&args[0]).and_then(run)
    } else {
        Err("usage: kf-user-ring-probe read-write|read-only|write-attempt".into())
    };
    match result {
        Ok(()) => println!("USER_RING_PROBE=PASS"),
        Err(e) => {
            eprintln!("USER_RING_PROBE=FAIL {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_ab_changes_only_gpu_mapping_permission() {
        assert_eq!(
            commands(Mode::ReadWrite, 0x200000000, 0x200010000),
            commands(Mode::ReadOnly, 0x200000000, 0x200010000)
        );
        assert!(!Mode::ReadWrite.perm().read_only);
        assert!(Mode::ReadOnly.perm().read_only);
        assert!(Mode::WriteAttempt.perm().read_only);
    }

    #[test]
    fn attempted_store_is_inside_the_private_mapping_before_real_tail() {
        let words = commands(Mode::WriteAttempt, 0x200000000, 0x200010000).unwrap();
        assert_eq!(words.len(), 14);
        assert_eq!(words[1], SENTINEL as u32);
        assert_eq!(words[3], ATTEMPT);
        assert_eq!(words[8], 0x10000);
        assert_eq!(words[10], 1);
        assert!(PB + 4 * words.len() as u64 <= SENTINEL);
        assert!(SENTINEL + 4 <= BYTES);
    }

    #[test]
    fn malformed_mode_cannot_launch_hardware() {
        assert!(Mode::parse("anything").is_err());
    }
}
