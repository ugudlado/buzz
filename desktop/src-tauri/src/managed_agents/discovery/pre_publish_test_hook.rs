//! Test-only seam: a callback invoked between discovery's directory scan and
//! its registry publish, so tests can land a `save_and_warm`/`delete_and_warm`
//! in exactly the window the stale-snapshot bug lived in — through the REAL
//! `discover_acp_runtimes_from` call path, not a hand-called seam.

use std::sync::{Mutex, OnceLock};

pub(crate) type Hook = Box<dyn Fn() + Send>;

fn cell() -> &'static Mutex<Option<Hook>> {
    static CELL: OnceLock<Mutex<Option<Hook>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Install (or clear, with `None`) the hook. Callers must serialize via
/// `registry_test_lock` — the hook is process-global.
pub(crate) fn set(hook: Option<Hook>) {
    *cell().lock().unwrap_or_else(|e| e.into_inner()) = hook;
}

pub(crate) fn run() {
    if let Some(hook) = cell().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        hook();
    }
}

/// RAII guard: installs the pre-publish hook, clears it on drop (even on
/// panic) so a failing test cannot poison later ones.
pub(crate) struct PrePublishHookGuard;

impl PrePublishHookGuard {
    pub(crate) fn install(hook: Hook) -> Self {
        set(Some(hook));
        PrePublishHookGuard
    }
}

impl Drop for PrePublishHookGuard {
    fn drop(&mut self) {
        set(None);
    }
}
