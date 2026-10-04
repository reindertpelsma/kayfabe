//! An authored, optional, read-only PTIMER view. No guest bytes or addresses enter this API.
use crate::{
    ABI_DECODE_FAILED, CpuViewRelease, HostRm, MapNode, NOT_ON_THIS_RUNG, RmError, ViewAccess,
    region_error,
};
use kf_linux_raw::{
    Backing, CachePolicy, CharDevice, HostOffset, HostPageSize, HostSpan, VolatileRegion,
};

/// Owns one mapped timer object. Drop removes the CPU mapping before releasing RM's view and
/// object; its borrow prevents the RM session from being destroyed first.
#[derive(Debug)]
pub struct TimerWindow<'a> {
    owner: &'a HostRm,
    object: u32,
    cookie: u64,
    region: Option<VolatileRegion>,
    _node: CharDevice,
    base: u32,
    layout: kf_abi::timer::TimerLayout,
}

impl TimerWindow<'_> {
    pub fn bar0_base(&self) -> u32 {
        self.base
    }
    pub fn layout(&self) -> kf_abi::timer::TimerLayout {
        self.layout
    }
    pub fn view(&self) -> HostSpan {
        self.region
            .as_ref()
            .expect("live timer mapping")
            .host_span()
    }
    /// Bounded coherent read, for native validation only; guests read the same host page directly.
    pub fn read_ns(&self) -> Result<u64, RmError> {
        let region = self.region.as_ref().expect("live timer mapping");
        kf_abi::submit::ptimer_sample(self.layout.time_high, self.layout.time_low, |off| {
            region
                .load_u32(HostOffset::new(off))
                .map_err(|e| region_error(&e))
        })
        .map_err(|_| RmError::Other(ABI_DECODE_FAILED))
    }
}
impl Drop for TimerWindow<'_> {
    fn drop(&mut self) {
        drop(self.region.take());
        if let Err(e) = self.owner.release_cpu_view(CpuViewRelease {
            h_memory: self.object,
            p_linear_address: self.cookie,
        }) {
            eprintln!("kf-host: timer CPU view release refused: {e:?}");
        }
        if let Err(e) = self.owner.free(self.object) {
            eprintln!("kf-host: timer object release refused: {e:?}");
        }
    }
}

impl HostRm {
    /// Open a PTIMER register view using the host driver's exact measured SDK layout and the
    /// GPU's NON_PRIVILEGED offset query. This runs during realize, before any vCPU exists.
    /// Only a 4 KiB host page is supported: larger host pages would expose extra registers.
    pub fn open_timer(&self) -> Result<TimerWindow<'_>, RmError> {
        let layout = kf_abi::timer::layout(self.host_abi().version())
            .ok_or(RmError::Other(NOT_ON_THIS_RUNG))?;
        let page = HostPageSize::query();
        if page.bytes() != 4096 {
            return Err(RmError::Other(NOT_ON_THIS_RUNG));
        }
        let mut payload = [0u8; 4];
        self.raw_control(
            self.subdevice(),
            kf_abi::timer::REGISTER_OFFSET,
            &mut payload,
        )?;
        let base = u32::from_le_bytes(payload);
        if base == 0 || !base.is_multiple_of(4096) || base.checked_add(4096).is_none() {
            return Err(RmError::Other(ABI_DECODE_FAILED));
        }
        let object = self.raw_alloc(
            self.subdevice(),
            self.mint(),
            kf_abi::submit::NV01_TIMER,
            None,
            &mut [],
        )?;
        self.remember(object, self.subdevice());
        let result = (|| {
            let (node, cookie) = self.arm_cpu_view(
                MapNode::Gpu,
                object,
                0,
                layout.register_bytes,
                ViewAccess::ReadOnly,
            )?;
            let region = match VolatileRegion::map_read_only(
                Backing::DeviceFile { fd: node.as_fd() },
                4096,
                CachePolicy::Uncached,
                page,
            ) {
                Ok(region) => region,
                Err(e) => {
                    let _ = self.release_cpu_view(CpuViewRelease {
                        h_memory: object,
                        p_linear_address: cookie,
                    });
                    return Err(region_error(&e));
                }
            };
            Ok(TimerWindow {
                owner: self,
                object,
                cookie,
                region: Some(region),
                _node: node,
                base,
                layout,
            })
        })();
        if result.is_err() {
            let _ = self.free(object);
        }
        result
    }
}
