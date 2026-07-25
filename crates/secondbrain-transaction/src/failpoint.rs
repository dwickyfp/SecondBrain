//! Debug-build crash injection used by durability acceptance tests.

#[cfg(debug_assertions)]
use std::{cell::RefCell, io};

#[cfg(debug_assertions)]
thread_local! { static LOCAL: RefCell<Option<String>> = const { RefCell::new(None) }; }

/// Select a returned-error failpoint for the current thread.
#[cfg(debug_assertions)]
pub fn set(name: Option<&str>) {
    LOCAL.with(|slot| *slot.borrow_mut() = name.map(str::to_owned));
}

pub(crate) fn hit(name: &str) -> std::io::Result<()> {
    #[cfg(debug_assertions)]
    {
        let local = LOCAL.with(|slot| slot.borrow().clone());
        let selected = local.or_else(|| std::env::var("SECONDBRAIN_TEST_FAILPOINT").ok());
        if selected.as_deref() == Some(name) {
            if std::env::var_os("SECONDBRAIN_TEST_FAILPOINT_ABORT").is_some() {
                std::process::abort();
            }
            return Err(io::Error::other(format!("injected failpoint: {name}")));
        }
    }
    Ok(())
}
