//! One native store-view instance shared by the device's USER and Translated readers.
//! Native mapping calls occur only in preparation, outside source-placement guards.
use crate::{
    mem::userread::{self, Refusal, Store},
    storeview::{self, ArmError, Backend, Error, Full, Owner, Windowed},
};
use kf_chan::tmode::Span;
use kf_host::{HostRm, MapNode, ViewAccess};
use kf_linux_raw::{Backing, CachePolicy, CharDevice, HostOffset, HostPageSize, VolatileRegion};
use std::sync::{Arc, atomic::Ordering};

#[derive(Clone, Copy)]
struct Native {
    rm: &'static HostRm,
    memory: u32,
    bound: u64,
}
struct View {
    cpu: Option<VolatileRegion>,
    _node: CharDevice,
    cookie: Option<u64>,
}
impl Backend for Native {
    type View = View;
    fn arm(&self, off: u64, len: u64) -> Result<View, ArmError<View>> {
        if len == 0 || off.checked_add(len).is_none_or(|e| e > self.bound) {
            return Err(ArmError { retained: None });
        }
        let (node, cookie) = self
            .rm
            .arm_cpu_view(MapNode::Gpu, self.memory, off, len, ViewAccess::ReadWrite)
            .map_err(|_| ArmError { retained: None })?;
        let cpu = VolatileRegion::map(
            Backing::DeviceFile { fd: node.as_fd() },
            len,
            CachePolicy::Uncached,
            HostPageSize::query(),
        );
        match cpu {
            Ok(cpu) => Ok(View {
                cpu: Some(cpu),
                _node: node,
                cookie: Some(cookie),
            }),
            Err(_) => Err(ArmError {
                retained: Some(View {
                    cpu: None,
                    _node: node,
                    cookie: Some(cookie),
                }),
            }),
        }
    }
    fn release(&self, v: &mut View) -> Result<(), Error> {
        v.cpu.take();
        if let Some(cookie) = v.cookie {
            self.rm
                .release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: self.memory,
                    p_linear_address: cookie,
                })
                .map_err(|_| Error::Release)?;
            v.cookie = None;
        }
        Ok(())
    }
    fn read(&self, v: &View, off: u64, out: &mut [u8]) -> Result<(), Error> {
        if out.len() > storeview::ACCESS {
            return Err(Error::Bounds);
        }
        v.cpu
            .as_ref()
            .ok_or(Error::Read)?
            .copy_out(HostOffset::new(off), out)
            .map_err(|_| Error::Read)
    }
    fn write(&self, v: &mut View, off: u64, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > storeview::ACCESS {
            return Err(Error::Bounds);
        }
        v.cpu
            .as_mut()
            .ok_or(Error::Write)?
            .write_data_small(HostOffset::new(off), bytes)
            .map_err(|_| Error::Write)
    }
}
enum Mode {
    Full(Full<Native>),
    Windowed(Windowed<Native>),
}
pub(crate) struct Shared {
    mode: Mode,
    native: Native,
    windows: usize,
}
impl Shared {
    pub(crate) fn new(
        rm: &'static HostRm,
        memory: u32,
        bound: u64,
        full: bool,
        windows: u32,
    ) -> Result<Self, String> {
        let native = Native { rm, memory, bound };
        let windows = if windows == 0 { 256 } else { windows as usize };
        if windows > 65536 || bound == 0 {
            return Err("STORE_VIEW_CONFIGURATION: invalid bound/window budget".into());
        }
        let mode = if full {
            match Full::new(native, bound) {
                Ok(f) => Mode::Full(f),
                Err((b, error)) => {
                    if let Some(mut v) = error.retained {
                        if b.release(&mut v).is_err() {
                            std::mem::forget(v);
                        }
                    }
                    return Err(
                        "STORE_VIEW_FULL_MAP: explicit full-store BAR1 mapping refused at realize"
                            .into(),
                    );
                }
            }
        } else {
            Mode::Windowed(
                Windowed::new(native, bound, windows)
                    .map_err(|e| format!("STORE_VIEW_WINDOWED: {e:?}"))?,
            )
        };
        eprintln!(
            "kf3: STORE_VIEW mode={} bound={bound:#x} window_budget={windows} access_max=256",
            if full { "FULL" } else { "WINDOWED" }
        );
        Ok(Self {
            mode,
            native,
            windows,
        })
    }
    pub(crate) fn summary(&self) -> String {
        match &self.mode {
            Mode::Full(_) => format!("STORE_VIEW mode=FULL live=1 bound={:#x}", self.native.bound),
            Mode::Windowed(t) => {
                let c = &t.counters;
                format!(
                    "STORE_VIEW mode=WINDOWED live={} arms={} releases={} rm_refused={} retry_ok={} fallback={} release_refused={}",
                    t.live(),
                    c.arms.load(Ordering::Relaxed),
                    c.releases.load(Ordering::Relaxed),
                    c.arm_refused.load(Ordering::Relaxed),
                    c.retry_ok.load(Ordering::Relaxed),
                    c.fallback.load(Ordering::Relaxed),
                    c.release_refused.load(Ordering::Relaxed)
                )
            }
        }
    }
    fn read(&self, off: u64, out: &mut [u8]) -> Result<(), Error> {
        match &self.mode {
            Mode::Full(f) => f.read(off, out),
            Mode::Windowed(t) => t.read(off, out),
        }
    }
}
fn refusal(e: Error) -> Refusal {
    match e {
        Error::Retry => Refusal::Busy,
        Error::Bounds => Refusal::Bounds,
        Error::Allocation => Refusal::Allocation,
        _ => Refusal::Store,
    }
}
struct Ephemeral {
    off: u64,
    len: u64,
    view: View,
    ready: bool,
}
/// A channel identity and prepared fallback views; no channel-owned persistent cache.
pub(crate) struct Reader {
    shared: &'static Shared,
    owner: Arc<Owner>,
    fallback: Vec<Ephemeral>,
}
impl Reader {
    pub(crate) fn new(shared: &'static Shared) -> Self {
        Self {
            shared,
            owner: Arc::new(Owner::default()),
            fallback: Vec::new(),
        }
    }
    pub(crate) fn release(&mut self) -> Result<(), Refusal> {
        self.owner.retire();
        self.clear_fallback()
    }
    fn clear_fallback(&mut self) -> Result<(), Refusal> {
        while let Some(v) = self.fallback.last_mut() {
            v.ready = false;
            self.shared.native.release(&mut v.view).map_err(refusal)?;
            self.fallback.pop();
        }
        Ok(())
    }
    pub(crate) fn stats(&self) -> (u64, u64, usize) {
        (self.owner.live() as u64, 0, self.shared.windows)
    }
    /// Translated source reader, invoked after its placement guard was dropped.
    pub(crate) fn read_prepared(&mut self, off: u64, out: &mut [u8]) -> Result<(), Refusal> {
        if out.len() > userread::MAX_BYTES {
            return Err(Refusal::Bounds);
        }
        self.prepare(&[Span {
            ram: false,
            off,
            len: out.len() as u64,
            perm: kf_host::MapPerm::READ_WRITE,
        }])?;
        self.copy(off, out)
    }
}
impl Store for Reader {
    fn prepare(&mut self, spans: &[Span]) -> Result<(), Refusal> {
        if spans.len() > userread::MAX_BYTES / 4096 + 1 {
            return Err(Refusal::Bounds);
        }
        let mut total = 0u64;
        for span in spans {
            total = total.checked_add(span.len).ok_or(Refusal::Bounds)?;
            if total > userread::MAX_BYTES as u64
                || (!span.ram
                    && span
                        .off
                        .checked_add(span.len)
                        .is_none_or(|e| e > self.shared.native.bound))
            {
                return Err(Refusal::Bounds);
            }
        }
        self.clear_fallback()?;
        let Mode::Windowed(t) = &self.shared.mode else {
            return Ok(());
        };
        for s in spans.iter().filter(|s| !s.ram) {
            let end = s.off + s.len;
            let mut at = s.off;
            while at < end {
                let base = at / storeview::WINDOW * storeview::WINDOW;
                match t.prepare(&self.owner, at) {
                    Ok(()) => (),
                    Err(Error::Arm) => {
                        if !self.fallback.iter().any(|v| v.off == base) {
                            let len = storeview::WINDOW.min(self.shared.native.bound - base);
                            self.fallback
                                .try_reserve(1)
                                .map_err(|_| Refusal::Allocation)?;
                            match self.shared.native.arm(base, len) {
                                Ok(view) => self.fallback.push(Ephemeral {
                                    off: base,
                                    len,
                                    view,
                                    ready: true,
                                }),
                                Err(e) => {
                                    if let Some(view) = e.retained {
                                        self.fallback.push(Ephemeral {
                                            off: base,
                                            len,
                                            view,
                                            ready: false,
                                        });
                                    }
                                    return Err(Refusal::Store);
                                }
                            }
                            t.counters.fallback.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => return Err(refusal(e)),
                }
                at = base + storeview::WINDOW.min(self.shared.native.bound - base);
            }
        }
        Ok(())
    }
    fn copy(&self, off: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
        if bytes.len() > userread::MAX_BYTES
            || off
                .checked_add(bytes.len() as u64)
                .is_none_or(|e| e > self.shared.native.bound)
        {
            return Err(Refusal::Bounds);
        }
        let mut done = 0;
        while done < bytes.len() {
            let at = off + done as u64;
            let n = (bytes.len() - done)
                .min(storeview::ACCESS)
                .min((storeview::WINDOW - at % storeview::WINDOW) as usize);
            let out = &mut bytes[done..done + n];
            if let Some(v) = self
                .fallback
                .iter()
                .find(|v| v.ready && at >= v.off && at + n as u64 <= v.off + v.len)
            {
                self.shared
                    .native
                    .read(&v.view, at - v.off, out)
                    .map_err(refusal)?;
            } else {
                self.shared.read(at, out).map_err(refusal)?;
            }
            done += n;
        }
        Ok(())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // No native call on Drop: some error paths drop under metadata guards.
        // Explicit release is the caller's cleanup contract. A live fallback
        // cookie must survive until session close if that contract failed.
        self.owner.retire();
        for v in self.fallback.drain(..) {
            if v.view.cookie.is_some() {
                std::mem::forget(v);
            }
        }
    }
}
