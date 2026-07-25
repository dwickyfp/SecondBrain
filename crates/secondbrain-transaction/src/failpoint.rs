//! Debug-build crash injection used by durability acceptance tests.

use std::{cell::RefCell, io};

thread_local! {
    static LOCAL: RefCell<Option<String>> = const { RefCell::new(None) };
    static ABORT: RefCell<bool> = const { RefCell::new(false) };
}

/// Select a returned-error failpoint for the current thread.
pub fn set(name: Option<&str>) {
    LOCAL.with(|slot| *slot.borrow_mut() = name.map(str::to_owned));
}

/// Makes a programmatically selected failpoint abort this test process.
///
/// This is deliberately not connected to a release-build environment variable.
#[doc(hidden)]
pub fn set_abort(abort: bool) {
    ABORT.with(|slot| *slot.borrow_mut() = abort);
}

pub(crate) fn hit(_name: &str) -> std::io::Result<()> {
    let local = LOCAL.with(|slot| slot.borrow().clone());
    #[cfg(debug_assertions)]
    let selected = local.or_else(|| std::env::var("SECONDBRAIN_TEST_FAILPOINT").ok());
    #[cfg(not(debug_assertions))]
    let selected = local;
    if selected.as_deref() == Some(_name) {
        if ABORT.with(|slot| *slot.borrow()) {
            std::process::abort();
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("SECONDBRAIN_TEST_FAILPOINT_ABORT").is_some() {
            std::process::abort();
        }
        return Err(io::Error::other(format!("injected failpoint: {_name}")));
    }
    Ok(())
}
