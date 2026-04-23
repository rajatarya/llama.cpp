//! Opaque `LlamaXetSession` handle exposed to C callers.
//!
//! Internally owns an `xet::xet_session::XetSession` plus any auth/
//! endpoint strings supplied at construction. The strings are stashed
//! on the handle rather than passed to the session-builder here because
//! xet-core wires authentication at the *download group* level, not at
//! the session level. The strings get consumed by the group builder in
//! Task 6.
//!
//! Lifetime rules (enforced by convention, not compiler — this is FFI):
//!   - Handles returned by `llama_xet_session_new` must be freed via
//!     `llama_xet_session_free` exactly once. No aliasing across threads
//!     without external synchronization.
//!   - `llama_xet_session_abort` is safe to call from a signal handler.
//!   - All three FFI functions are NULL-tolerant on the session pointer
//!     (free/abort silently no-op; new never receives a session).

use std::ffi::CStr;
use std::os::raw::c_char;

use xet::xet_session::{XetSession, XetSessionBuilder};

use crate::error;

/// Opaque handle. C sees `typedef struct LlamaXetSession LlamaXetSession;`.
/// Fields are Rust-only; cbindgen does not peek inside (no `#[repr(C)]`).
pub struct LlamaXetSession {
    pub(crate) inner:             XetSession,
    pub(crate) endpoint:          Option<String>,
    pub(crate) bearer_token:      Option<String>,
    pub(crate) token_refresh_url: Option<String>,
}

fn cstr_to_opt_string(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller promises p is a valid NUL-terminated C string
    // or NULL. Non-UTF-8 input yields None.
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

pub(crate) fn new_inner(
    endpoint:          *const c_char,
    bearer_token:      *const c_char,
    token_refresh_url: *const c_char,
) -> Option<Box<LlamaXetSession>> {
    error::clear();

    let endpoint          = cstr_to_opt_string(endpoint);
    let bearer_token      = cstr_to_opt_string(bearer_token);
    let token_refresh_url = cstr_to_opt_string(token_refresh_url);

    match XetSessionBuilder::new().build() {
        Ok(inner) => Some(Box::new(LlamaXetSession {
            inner,
            endpoint,
            bearer_token,
            token_refresh_url,
        })),
        Err(e) => {
            error::set(format!("XetSessionBuilder::build failed: {e}"));
            None
        }
    }
}

pub(crate) fn abort_inner(session: *mut LlamaXetSession) {
    if session.is_null() {
        return;
    }
    // SAFETY: pointer obtained from Box::into_raw in new_inner.
    // Reading a shared reference is safe; sigint_abort takes &self.
    let s = unsafe { &*session };
    let _ = s.inner.sigint_abort();
}

pub(crate) fn free_inner(session: *mut LlamaXetSession) {
    if session.is_null() {
        return;
    }
    // SAFETY: pointer obtained from Box::into_raw in new_inner;
    // caller has not aliased it or freed it already (enforced by
    // convention — see module docs).
    unsafe { drop(Box::from_raw(session)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_free_roundtrip_with_nulls() {
        let s = new_inner(std::ptr::null(), std::ptr::null(), std::ptr::null());
        assert!(s.is_some(), "session_new should succeed with NULL args");
        let raw = Box::into_raw(s.unwrap());
        free_inner(raw);
    }

    #[test]
    fn new_captures_fields() {
        let ep  = std::ffi::CString::new("https://cas.example").unwrap();
        let tok = std::ffi::CString::new("hf_xxx").unwrap();
        let ref_url = std::ffi::CString::new("https://hf/api/.../xet-read-token/main").unwrap();

        let s = new_inner(ep.as_ptr(), tok.as_ptr(), ref_url.as_ptr()).unwrap();
        assert_eq!(s.endpoint.as_deref(),          Some("https://cas.example"));
        assert_eq!(s.bearer_token.as_deref(),      Some("hf_xxx"));
        assert_eq!(s.token_refresh_url.as_deref(), Some("https://hf/api/.../xet-read-token/main"));

        free_inner(Box::into_raw(s));
    }

    #[test]
    fn free_null_is_safe() {
        free_inner(std::ptr::null_mut());
    }

    #[test]
    fn abort_null_is_safe() {
        abort_inner(std::ptr::null_mut());
    }

    #[test]
    fn abort_live_session_is_safe() {
        let s = new_inner(std::ptr::null(), std::ptr::null(), std::ptr::null()).unwrap();
        let raw = Box::into_raw(s);
        abort_inner(raw);
        free_inner(raw);
    }
}
