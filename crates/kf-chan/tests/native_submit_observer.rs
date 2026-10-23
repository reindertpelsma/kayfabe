//! The measurement callback cannot suppress, duplicate or alter a native call.
#[path = "../src/submit_observer.rs"]
mod submit_observer;

use std::sync::Mutex;
use submit_observer::{Observer, Stamp, observe};

#[derive(Default)]
struct Log(Mutex<Vec<&'static str>>);
impl Observer for Log {
    fn before(&self) -> Stamp {
        self.0.lock().unwrap().push("begin");
        Stamp {
            first_ns: 10,
            born_ns: 1,
        }
    }
    fn after(&self, stamp: Stamp, ok: bool) {
        assert_eq!(stamp.first_ns, 10);
        assert_eq!(stamp.born_ns, 1);
        self.0.lock().unwrap().push(if ok { "ok" } else { "error" });
    }
}

#[test]
fn success_is_returned_unchanged_and_recorded_after_exactly_one_call() {
    let log = Log::default();
    let result: Result<u32, &str> = observe(Some(&log), || {
        log.0.lock().unwrap().push("native");
        Ok(71)
    });
    assert_eq!(result, Ok(71));
    assert_eq!(*log.0.lock().unwrap(), ["begin", "native", "ok"]);
}

#[test]
fn native_error_is_returned_unchanged_and_never_marked_success() {
    let log = Log::default();
    let result: Result<u32, &str> = observe(Some(&log), || {
        log.0.lock().unwrap().push("native");
        Err("original-native-error")
    });
    assert_eq!(result, Err("original-native-error"));
    assert_eq!(*log.0.lock().unwrap(), ["begin", "native", "error"]);
}

#[test]
fn disabled_success_calls_native_once() {
    let mut calls = 0;
    let result: Result<u32, &str> = observe(None, || {
        calls += 1;
        Ok(71)
    });
    assert_eq!((result, calls), (Ok(71), 1));
}

#[test]
fn disabled_error_calls_native_once_and_preserves_error() {
    let mut calls = 0;
    let result: Result<u32, &str> = observe(None, || {
        calls += 1;
        Err("original-native-error")
    });
    assert_eq!((result, calls), (Err("original-native-error"), 1));
}
