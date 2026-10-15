//! Native-attempt quota for the disabled Windows USER ring adapter.
//!
//! Charge the advertised runlist budget before any native channel allocation.
//! Token publication transfers that same charge under one caps lock. Guest
//! token retirement does not release it; only never-attempted cancellation or
//! confirmed native cleanup does. Dropping a ticket grants no cleanup credit.

use kf_core::{Plane, VmCaps};
use kf_trap::token::Route;
use std::sync::{Arc, Mutex};

struct State {
    charged: bool,
    attached: bool,
    token_retired: bool,
}

pub(super) struct NativeQuota {
    caps: Arc<Mutex<VmCaps>>,
    plane: &'static Plane<'static>,
    idx: u32,
    runlist: u32,
    state: Mutex<State>,
}

impl NativeQuota {
    pub(super) fn reserve(
        caps: Arc<Mutex<VmCaps>>,
        plane: &'static Plane<'static>,
        idx: u32,
    ) -> Result<Arc<Self>, String> {
        if plane.token_index.clamp(idx) != idx || idx as usize >= plane.tokens.len() {
            return Err("USER_RING_QUOTA_TOKEN_BOUNDS".into());
        }
        let runlist = plane.token_index.runlist_of(idx);
        caps.lock()
            .map_err(|_| "USER_RING_CAPS_POISONED")?
            .acquire_channel(runlist)
            .map_err(|e| format!("USER_RING_QUOTA {e:?}"))?;
        Ok(Arc::new(Self {
            caps,
            plane,
            idx,
            runlist,
            state: Mutex::new(State {
                charged: true,
                attached: false,
                token_retired: false,
            }),
        }))
    }

    /// Metadata only. No native call, source read, or vCPU acquisition.
    pub(super) fn attach(&self, host_token: u32) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "USER_RING_QUOTA_POISONED")?;
        if !state.charged || state.attached {
            return Err("USER_RING_QUOTA_NOT_ATTACHABLE".into());
        }
        let mut caps = self.caps.lock().map_err(|_| "USER_RING_CAPS_POISONED")?;
        // Reuse Plane's normal publication checks without a second charge.
        // No other birth can observe the intermediate decrement under this lock.
        caps.release_channel(self.runlist);
        let result = self.plane.allocate_channel(
            &mut caps,
            self.idx,
            Route::Translated,
            host_token,
            kf_core::Owner::User,
        );
        if let Err(e) = result {
            // allocate_channel rolls back its own charge on token collision.
            // The native attempt still exists and must remain charged.
            caps.acquire_channel(self.runlist).map_err(|restore| {
                format!("USER_RING_QUOTA_RESTORE {restore:?}; publication {e:?}")
            })?;
            return Err(format!("USER_RING_TOKEN {e:?}"));
        }
        state.attached = true;
        Ok(())
    }

    /// Wake/token lifetime ends before native lifetime. Retain the charge.
    pub(super) fn retire_token(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.charged || !state.attached {
            return false;
        }
        if state.token_retired {
            return true;
        }
        let Ok(mut caps) = self.caps.lock() else {
            return false;
        };
        state.token_retired = self.plane.retire_channel_token(&mut caps, self.idx);
        state.token_retired
    }

    /// Native cleanup cannot drop a twin that a worker still owns. An error
    /// retains the native resources and charge; it is not cleanup permission.
    pub(super) fn prepare_native_cleanup(&self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "USER_RING_QUOTA_POISONED")?;
        if !state.charged {
            return Err("USER_RING_QUOTA_ALREADY_RELEASED".into());
        }
        if !state.attached || state.token_retired {
            return Ok(());
        }
        let mut caps = self.caps.lock().map_err(|_| "USER_RING_CAPS_POISONED")?;
        if !self.plane.retire_channel_token(&mut caps, self.idx) {
            return Err("USER_RING_TOKEN_STILL_OWNED".into());
        }
        state.token_retired = true;
        Ok(())
    }

    /// Caller must prove no native allocation was attempted, or native free
    /// succeeded. An error or elapsed time never authorizes this call.
    pub(super) fn release_confirmed(&self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "USER_RING_QUOTA_POISONED")?;
        if !state.charged {
            return Ok(());
        }
        if state.attached && !state.token_retired {
            return Err("USER_RING_QUOTA_TOKEN_NOT_RETIRED".into());
        }
        let mut caps = self.caps.lock().map_err(|_| "USER_RING_CAPS_POISONED")?;
        caps.release_channel(self.runlist);
        state.charged = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(cap: u32) -> (&'static Plane<'static>, Arc<Mutex<VmCaps>>) {
        let vmm = Box::leak(Box::new(kf_core::Vmm::default()));
        let plane = Box::leak(Box::new(Plane::new(vmm, 4, 3)));
        (
            plane,
            Arc::new(Mutex::new(VmCaps::from_declared(cap, 8, 8, 8))),
        )
    }

    #[test]
    fn uncertain_native_attempts_remain_charged_before_token_publication() {
        let (plane, caps) = setup(2);
        let a = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        let b = NativeQuota::reserve(caps.clone(), plane, 1).unwrap();
        assert!(NativeQuota::reserve(caps.clone(), plane, 2).is_err());
        // No native result/token: dropping the local ticket proves nothing.
        drop(a);
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 2);
        assert!(NativeQuota::reserve(caps.clone(), plane, 2).is_err());
        b.release_confirmed().unwrap();
        assert!(NativeQuota::reserve(caps.clone(), plane, 2).is_ok());
    }

    #[test]
    fn token_free_retains_native_quota_until_confirmed_cleanup() {
        let (plane, caps) = setup(1);
        let a = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        a.attach(77).unwrap();
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 1);
        assert!(a.retire_token());
        assert!(NativeQuota::reserve(caps.clone(), plane, 1).is_err());
        a.release_confirmed().unwrap();
        a.release_confirmed().unwrap();
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 0);
        assert!(NativeQuota::reserve(caps, plane, 1).is_ok());
    }

    #[test]
    fn failed_publication_restores_charge_and_does_not_replace_old_token() {
        let (plane, caps) = setup(2);
        plane
            .allocate_channel(
                &mut caps.lock().unwrap(),
                0,
                Route::Passthrough,
                11,
                kf_core::Owner::User,
            )
            .unwrap();
        let a = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        assert!(a.attach(22).is_err());
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 2);
        assert_eq!(plane.tokens[0].load().host_token, 11);
        assert!(!a.retire_token());
        a.release_confirmed().unwrap();
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 1);
    }

    #[test]
    fn a_charge_cannot_be_published_twice_or_after_release() {
        let (plane, caps) = setup(2);
        let a = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        a.attach(77).unwrap();
        assert!(a.attach(88).is_err());
        assert!(a.retire_token());
        a.release_confirmed().unwrap();
        assert!(a.attach(99).is_err());
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 0);
    }

    #[test]
    fn a_busy_worker_blocks_native_cleanup_and_quota_release() {
        let (plane, caps) = setup(1);
        let a = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        a.attach(77).unwrap();
        plane.tokens[0].ring(0);
        assert!(matches!(
            plane.tokens[0].claim(),
            kf_trap::token::Claim::Won(_)
        ));
        assert!(!a.retire_token());
        assert!(a.prepare_native_cleanup().is_err());
        assert!(a.release_confirmed().is_err());
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 1);
        plane.tokens[0].release(1, 1);
        a.prepare_native_cleanup().unwrap();
        a.release_confirmed().unwrap();
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 0);
    }

    #[test]
    fn repeated_retirement_cannot_retire_a_replacement_token() {
        let (plane, caps) = setup(2);
        let a = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        a.attach(77).unwrap();
        assert!(a.retire_token());
        let b = NativeQuota::reserve(caps.clone(), plane, 0).unwrap();
        b.attach(88).unwrap();
        assert!(a.retire_token());
        a.prepare_native_cleanup().unwrap();
        assert_eq!(plane.tokens[0].load().host_token, 88);
        a.release_confirmed().unwrap();
        assert_eq!(caps.lock().unwrap().live_on_runlist(0), 1);
        b.prepare_native_cleanup().unwrap();
        b.release_confirmed().unwrap();
    }
}
