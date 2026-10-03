// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The display broker, kf3's side** (`docs/design/V3_DISPLAY.md` §8, display step 3).
//!
//! The protocol, the connection machine and the frame ring are VMM-agnostic and live in
//! `kf-broker`; this file is what only the kf3 device knows:
//!
//! - **the frame backing** ([`BrokerSeat::frame`], the display WORKER): with `display-broker`
//!   set, each of the five frame slots is a sealed memfd (`kayfabe-display-frame`, sealed
//!   `SHRINK|GROW|SEAL`, never `WRITE`), mapped and page-locked with `cuMemHostRegister` so the
//!   existing asynchronous D2H scanout copy lands in it, plus a udmabuf over the same pages when
//!   `/dev/udmabuf` opened. The descriptors go into the ring and are NEVER closed while the
//!   device lives. Unset, nothing here exists and the console keeps `cuMemAllocHost` frames.
//! - **the relay seat** ([`BrokerSeat::start`] / [`BrokerSeat::ready`] / [`BrokerSeat::stop`],
//!   QEMU's MAIN LOOP only): the C device registers the socket and a timer through the hooks it
//!   passed, wakes the relay for socket readiness, the worker's frame eventfd and the timer, and
//!   injects the [`Input`] that comes back.
//!
//! - ★ **the guest's cursor** ([`BrokerSeat::cursor`], `OWNER_RULINGS.md` §O): the
//!   [`CursorShare`] the relay publishes the hover/grab mode in and the worker posts the guest's
//!   cursor image to (`kf_broker::cursor`).
//!
//! ⊘ The worker never touches the socket: it only publishes into the ring (CAS), posts the cursor
//! (a mutex both sides only TRY) and writes one non-blocking eventfd. No lock is waited on between
//! the worker and the main loop (the relay's mutex is taken only on the main loop; a status read
//! from elsewhere only tries it).

use crate::raw_unsafe::BrokerHooks;
use kf_broker::{
    CursorShare, FrameRing, Input, InstallRefusal, Relay, RelayConfig, SlotFds, UnixLink,
};
use kf_cuda::display::{DisplayGpu, Frame};
use kf_linux_raw::{
    Backing, CachePolicy, HostPageSize, HostProt, MappedRegion, Notifier, SharedRam, udmabuf_create,
};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::{Arc, Mutex};

/// ★ The broker's seat on the display plane.
pub struct BrokerSeat {
    ring: Arc<FrameRing>,
    /// The worker → main loop edge: one write per published frame.
    wake: Notifier,
    /// `/dev/udmabuf`, when it could be opened (`root:kvm 0660`): the dma-buf rungs.
    udmabuf: Option<std::fs::File>,
    /// The relay and the C device's hooks — the main loop only.
    relay: Mutex<Option<(Relay<UnixLink>, BrokerHooks)>>,
    /// ★ §O: the guest's cursor between the worker and the relay.
    cursor: Arc<CursorShare>,
}

impl std::fmt::Debug for BrokerSeat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerSeat")
            .field("udmabuf", &self.udmabuf.is_some())
            .finish_non_exhaustive()
    }
}

impl BrokerSeat {
    /// The seat over `ring` (five slots, feeding the broker).
    ///
    /// # Errors
    /// The frame eventfd could not be made.
    pub fn new(ring: Arc<FrameRing>) -> Result<BrokerSeat, String> {
        let wake = Notifier::create().map_err(|e| format!("display-broker eventfd: {e}"))?;
        let udmabuf = match kf_linux_raw::udmabuf_gate::open_device() {
            Ok(f) => {
                eprintln!(
                    "kf3: broker: /dev/udmabuf opened — frames can go as a dma-buf (rungs 1 and 1b)"
                );
                Some(f)
            }
            Err(e) => {
                eprintln!(
                    "kf3: broker: /dev/udmabuf cannot be opened ({e}) — frames go as shared memory \
                     (F_SHM) only; add QEMU's user to the kvm group for the dma-buf rungs"
                );
                None
            }
        };
        Ok(BrokerSeat {
            ring,
            wake,
            udmabuf,
            relay: Mutex::new(None),
            cursor: Arc::new(CursorShare::new()),
        })
    }

    /// ★ **Worker** (and the relay, through its own handle): the guest's cursor share — the mode
    /// the worker composes by, and the mailbox it posts the guest's cursor to (§O).
    #[must_use]
    pub fn cursor(&self) -> &CursorShare {
        &self.cursor
    }

    /// ★ **Worker**: a broker frame of at least `cap` bytes — whole host pages
    /// ([`kf_broker::frame_bytes`]) — for the FREE slot `slot`. The memfd is registered for the
    /// copy BEFORE its descriptors are installed, so the ring never names a backing the GPU does
    /// not write. ⊘ On an error the caller withdraws the HOST kind from the broker
    /// ([`FrameRing::withdraw`]`(Kind::Host)`, `display.rs` `broker_backing`) before any slot is
    /// refilled with other memory: the ring would otherwise go on offering a reallocated slot's
    /// previous backing, which the GPU no longer writes, and every other slot's frames to a broker
    /// the worker said would be shown nothing through host memory. (The VRAM kind — the GPU-copy
    /// rung, §8.11 — is withdrawn only by its own refusals.)
    ///
    /// ⊘ CORRECTED 2026-10-03 (the review of `v3-broker`): a size that was not a multiple of the
    /// host page was REFUSED, and 1920x1080x4 is not a multiple of 64 KiB — so on a host with
    /// 64 KiB pages (arm64) every broker frame was refused. It is now rounded up.
    ///
    /// # Errors
    /// By name: the memfd, the mapping, the registration (`cuMemHostRegister` on a memfd mapping
    /// is the one step not yet run on hardware), or the ring's refusal.
    pub fn frame(&self, gpu: &DisplayGpu, slot: usize, cap: usize) -> Result<Frame, String> {
        let page = HostPageSize::query();
        let bytes = kf_broker::frame_bytes(cap as u64, page.bytes())
            .ok_or_else(|| format!("a {cap}-byte frame does not round to whole host pages"))?;
        for _ in 0..3 {
            let ram = SharedRam::create_named(c"kayfabe-display-frame", bytes)
                .map_err(|e| format!("memfd: {e}"))?;
            let map = MappedRegion::map(
                Backing::SharedFile {
                    fd: ram.as_backing_fd(),
                    offset: 0,
                },
                bytes,
                HostProt::ReadWrite,
                CachePolicy::WriteBack,
                page,
            )
            .map_err(|e| format!("mapping the frame memfd: {e}"))?;
            let frame = gpu.frame_over(map).map_err(|(e, map)| {
                // never registered: the mapping can go
                drop(map);
                format!("cuMemHostRegister of the frame memfd: {e}")
            })?;
            let dmabuf = self.udmabuf.as_ref().and_then(|d| {
                udmabuf_create(d.as_fd(), &ram, page)
                    .map_err(|e| {
                        eprintln!(
                            "kf3: broker: UDMABUF_CREATE refused ({e}): this frame goes as F_SHM"
                        )
                    })
                    .ok()
            });
            let fds = SlotFds::new(ram, dmabuf).map_err(|e| format!("frame identity: {e}"))?;
            match self.ring.install(slot, fds) {
                Ok(()) => return Ok(frame),
                Err((InstallRefusal::IdCollision(id), _)) => {
                    // an inode number another slot already carries: make a new backing
                    eprintln!(
                        "kf3: broker: frame id {id} collides; recreating slot {slot}'s backing"
                    );
                    let _ = gpu.release_frame(frame);
                }
                Err((e, _)) => {
                    let _ = gpu.release_frame(frame);
                    return Err(format!("the frame ring refused slot {slot}: {e:?}"));
                }
            }
        }
        Err(format!("slot {slot}: three backings in a row collided"))
    }

    /// ★ **Worker**: a frame was published — one non-blocking eventfd write.
    pub fn frame_published(&self) {
        let _ = self.wake.signal();
    }

    /// The frame eventfd the C device watches (main loop).
    #[must_use]
    pub fn frame_fd(&self) -> i32 {
        self.wake.as_source_fd().as_raw_fd()
    }

    /// ★ **Main loop** (realize): start the relay — the path and `display-broker-uid` are
    /// checked, and the first attempt is ARMED on the relay's timer, so it runs from QEMU's main
    /// loop, after `-run-with user=`/`-runas` dropped privileges; a broker that is not up yet is
    /// retried in the background (never a startup dependency).
    ///
    /// # Errors
    /// A refused path, a `display-broker-uid` that is not -1 or a uid, or a second start.
    pub fn start(
        &self,
        path: &std::path::Path,
        extra_uid: i64,
        mut hooks: BrokerHooks,
        now_ms: u64,
    ) -> Result<(), String> {
        kf_linux_raw::check_socket_path(path).map_err(|e| format!("display-broker: {e}"))?;
        let extra_uid = kf_broker::broker_uid_property(extra_uid)?;
        let mut g = self
            .relay
            .lock()
            .map_err(|_| "display-broker: the relay lock is poisoned")?;
        if g.is_some() {
            return Err("display-broker: the relay is already started".into());
        }
        eprintln!(
            "kf3: broker: relay to {} (brokers accepted: uid 0, QEMU's effective uid at each \
             connect{}); no clipboard",
            path.display(),
            extra_uid.map_or_else(String::new, |x| format!(", display-broker-uid {x}"))
        );
        let mut relay = Relay::new(
            RelayConfig {
                path: path.to_path_buf(),
                extra_uid,
            },
            self.ring.clone(),
            UnixLink,
        )
        .with_cursor(self.cursor.clone());
        relay.start(now_ms, &mut hooks);
        *g = Some((relay, hooks));
        Ok(())
    }

    /// ★ **Main loop**: something is ready — `fd` is the socket (`rd`/`wr`), the frame eventfd,
    /// or `-1` for the timer. Input for the C device is appended to `out` (at most `cap`).
    /// Returns whether a broker is connected and ACTIVE (its activity counts as demand).
    pub fn ready(
        &self,
        fd: i32,
        rd: bool,
        wr: bool,
        now_ms: u64,
        out: &mut Vec<Input>,
        cap: usize,
    ) -> bool {
        let Ok(mut g) = self.relay.lock() else {
            return false;
        };
        let Some((relay, hooks)) = g.as_mut() else {
            return false;
        };
        if fd >= 0 && fd == self.frame_fd() {
            let _ = self.wake.drain();
            relay.on_frame(now_ms, hooks);
        } else if fd >= 0 && Some(fd) == relay.socket_fd() {
            relay.on_socket(now_ms, rd, wr, hooks, out, cap);
        } else {
            relay.on_timer(now_ms, hooks);
        }
        relay.active()
    }

    /// ★ **Main loop**, device exit: unwatch, close, no timer.
    pub fn stop(&self) {
        if let Ok(mut g) = self.relay.lock()
            && let Some((mut relay, mut hooks)) = g.take()
        {
            relay.stop(&mut hooks);
        }
    }

    /// The relay's status fragment — never waits for the main loop.
    #[must_use]
    pub fn status(&self) -> String {
        match self.relay.try_lock() {
            Ok(g) => g
                .as_ref()
                .map_or_else(|| "broker[not started]".into(), |(r, _)| r.status()),
            Err(_) => "broker[busy]".into(),
        }
    }
}
