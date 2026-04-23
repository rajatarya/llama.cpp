//! llama-xet: thin C ABI over xet-core's XetSession for consumption by
//! llama.cpp's `common/` download path.
//!
//! Naming gotcha: xet-core's download crate has three different names.
//!   - On-disk directory:  xet_pkg/
//!   - Cargo package name: hf-xet   (`[dependencies]` resolves on this)
//!   - Rust library name:  xet      (used in `use` — set via `[lib] name`)
//! So our dep is `hf-xet = {...}` and our import is
//! `use xet::xet_session::XetSession`.

mod error;
mod session;

pub use session::LlamaXetSession;

use std::ffi::c_char;

/// Returns a static, NUL-terminated string identifying the pinned
/// xet-core revision this build links against. The returned pointer
/// is static-lifetime; the caller must NOT free it.
///
/// Intended for `llama-cli --version` / diagnostic output so a user
/// can tell which xet-core snapshot their binary uses.
#[no_mangle]
pub extern "C" fn llama_xet_version() -> *const c_char {
    concat!("xet-core@b43c0aec\0").as_ptr() as *const c_char
}

/// Returns a pointer to the current thread's last-error C string,
/// or a pointer to an empty static C string if no error is set.
///
/// # Safety
///
/// The returned pointer is valid only until the next llama-xet FFI
/// call on the same thread. Do not free it.
#[no_mangle]
pub extern "C" fn llama_xet_last_error() -> *const c_char {
    error::get_ptr()
}

/// Creates a new XetSession. The three string arguments may be NULL.
///
/// - `endpoint`: CAS URL base. If NULL, xet-core resolves it from the
///   token-refresh response.
/// - `bearer_token`: HF bearer token. If NULL, anonymous access is
///   attempted (will fail for private repos).
/// - `token_refresh_url`: Full URL to the HF `xet-read-token/{rev}`
///   endpoint. If NULL, xet-core uses the static bearer token for
///   the lifetime of the session (refresh disabled).
///
/// Returns NULL on failure. Call `llama_xet_last_error` for the
/// diagnostic message. On success, caller owns the returned handle
/// and must eventually free it with `llama_xet_session_free`.
#[no_mangle]
pub extern "C" fn llama_xet_session_new(
    endpoint:          *const c_char,
    bearer_token:      *const c_char,
    token_refresh_url: *const c_char,
) -> *mut LlamaXetSession {
    match session::new_inner(endpoint, bearer_token, token_refresh_url) {
        Some(b) => Box::into_raw(b),
        None    => std::ptr::null_mut(),
    }
}

/// Frees a session handle previously returned by `llama_xet_session_new`.
/// Calling with NULL is a safe no-op. Must be called at most once per
/// handle; double-free is undefined behavior.
#[no_mangle]
pub extern "C" fn llama_xet_session_free(session: *mut LlamaXetSession) {
    session::free_inner(session);
}

/// Requests cancellation of any in-flight downloads on the given
/// session. Safe to call from a signal handler and safe to call
/// with NULL. The pending download call will return a "cancelled"
/// error shortly after.
#[no_mangle]
pub extern "C" fn llama_xet_session_abort(session: *mut LlamaXetSession) {
    session::abort_inner(session);
}

// Temporary — kept until Task 5 wraps XetSession for real so the
// xet-core symbol is referenced. Without this, rustc may eliminate
// the hf-xet dep at link time since nothing else uses it yet.
#[allow(dead_code)]
fn _probe_xet_session_exists() -> &'static str {
    std::any::type_name::<xet::xet_session::XetSession>()
}
