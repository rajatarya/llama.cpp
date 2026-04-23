//! llama-xet: thin C ABI over xet-core's XetSession for consumption by
//! llama.cpp's `common/` download path.
//!
//! Naming gotcha: xet-core's download crate has three different names.
//!   - On-disk directory:  xet_pkg/
//!   - Cargo package name: hf-xet   (`[dependencies]` resolves on this)
//!   - Rust library name:  xet      (used in `use` — set via `[lib] name`)
//! So our dep is `hf-xet = {...}` and our import is
//! `use xet::xet_session::XetSession`.

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

// Temporary — kept until Task 4 adds real FFI so the xet-core
// symbol is referenced. Without this, rustc may eliminate the
// hf-xet dep at link time since nothing else uses it yet.
#[allow(dead_code)]
fn _probe_xet_session_exists() -> &'static str {
    std::any::type_name::<xet::xet_session::XetSession>()
}
