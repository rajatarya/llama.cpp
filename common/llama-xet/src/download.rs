//! Batch download FFI: `llama_xet_download_files`.
//!
//! Wires C descriptors onto `XetSession::new_file_download_group()` →
//! `download_file_to_path_blocking()` → `finish_blocking()`. Progress
//! polling runs on a helper thread and invokes the user's C callback
//! at a fixed cadence (250 ms) until the download completes.

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use xet::xet_session::{HeaderMap, XetFileInfo};

use crate::error;
use crate::session::LlamaXetSession;

/// C-visible file descriptor. All pointers are non-NUL-terminated-
/// sensitive: `hash` and `dest_path` must be NUL-terminated UTF-8
/// C strings. `file_size` of 0 means "unknown" — xet-core fills in
/// from the metadata fetch during download.
#[repr(C)]
pub struct LlamaXetFileInfo {
    pub hash:      *const c_char,
    pub file_size: u64,
    pub dest_path: *const c_char,
}

/// Progress callback type. NULL means "no callback wanted".
///
/// `user_data` is the opaque pointer the caller passed to
/// `llama_xet_download_files`. `completed_bytes` and `total_bytes`
/// are cumulative across all files in the batch.
pub type LlamaXetProgressFn = Option<
    extern "C" fn(user_data: *mut c_void, completed_bytes: u64, total_bytes: u64),
>;

/// Success.
pub const LXET_OK: i32 = 0;
/// NULL pointer or otherwise malformed input.
pub const LXET_ERR_INVALID_ARG: i32 = 1;
/// Authentication/authorization failure (missing/expired token, 401, 403).
pub const LXET_ERR_AUTH: i32 = 2;
/// Network error (DNS, connect, timeout, mid-stream failure).
pub const LXET_ERR_NETWORK: i32 = 3;
/// Chunk integrity / hash-mismatch / data corruption.
pub const LXET_ERR_INTEGRITY: i32 = 4;
/// User cancelled via `llama_xet_session_abort` or SIGINT.
pub const LXET_ERR_CANCELLED: i32 = 5;
/// Any other failure.
pub const LXET_ERR_OTHER: i32 = 99;

fn cstr_to_string(p: *const c_char, what: &str) -> Result<String, i32> {
    if p.is_null() {
        error::set(format!("{what} pointer is NULL"));
        return Err(LXET_ERR_INVALID_ARG);
    }
    match unsafe { CStr::from_ptr(p) }.to_str() {
        Ok(s)  => Ok(s.to_owned()),
        Err(_) => {
            error::set(format!("{what} is not valid UTF-8"));
            Err(LXET_ERR_INVALID_ARG)
        }
    }
}

fn classify_error(msg: &str) -> i32 {
    let lower = msg.to_lowercase();
    if lower.contains("cancel")      { return LXET_ERR_CANCELLED; }
    if lower.contains("auth")
        || lower.contains("401")
        || lower.contains("403")     { return LXET_ERR_AUTH; }
    if lower.contains("integrity")
        || lower.contains("hash")
        || lower.contains("mismatch"){ return LXET_ERR_INTEGRITY; }
    if lower.contains("timeout")
        || lower.contains("connect")
        || lower.contains("network") { return LXET_ERR_NETWORK; }
    LXET_ERR_OTHER
}

pub(crate) fn download_inner(
    session:     *mut LlamaXetSession,
    files:       *const LlamaXetFileInfo,
    file_count:  usize,
    progress_cb: LlamaXetProgressFn,
    user_data:   *mut c_void,
) -> i32 {
    error::clear();

    if session.is_null() {
        error::set("session is NULL");
        return LXET_ERR_INVALID_ARG;
    }
    if files.is_null() && file_count > 0 {
        error::set("files is NULL but file_count > 0");
        return LXET_ERR_INVALID_ARG;
    }
    if file_count == 0 {
        return LXET_OK;   // trivially successful
    }

    // SAFETY: session pointer from Box::into_raw; caller agrees not to
    // alias. We only take a shared reference.
    let s = unsafe { &*session };

    // Marshal C descriptors → (XetFileInfo, PathBuf) pairs.
    let slice = unsafe { std::slice::from_raw_parts(files, file_count) };
    let mut items: Vec<(XetFileInfo, PathBuf)> = Vec::with_capacity(file_count);
    for (i, f) in slice.iter().enumerate() {
        let hash  = match cstr_to_string(f.hash,      &format!("files[{i}].hash"))      { Ok(v) => v, Err(c) => return c };
        let dest  = match cstr_to_string(f.dest_path, &format!("files[{i}].dest_path")) { Ok(v) => v, Err(c) => return c };
        let info  = XetFileInfo {
            hash,
            file_size: if f.file_size == 0 { None } else { Some(f.file_size) },
            sha256:    None,
        };
        items.push((info, PathBuf::from(dest)));
    }

    // Build the download group with whatever auth the session was
    // configured with. xet-core wires auth at the group level, so we
    // push the session's stashed strings into the builder here.
    let mut gb = match s.inner.new_file_download_group() {
        Ok(b)  => b,
        Err(e) => {
            let msg = format!("new_file_download_group: {e}");
            error::set(&msg);
            return classify_error(&msg);
        }
    };
    if let Some(ep) = s.endpoint.as_deref() {
        gb = gb.with_endpoint(ep);
    }
    if let Some(url) = s.token_refresh_url.as_deref() {
        gb = gb.with_token_refresh_url(url, HeaderMap::new());
    } else if let Some(tok) = s.bearer_token.as_deref() {
        // Static token, far-future expiry. xet-core treats this as
        // "refresh disabled".
        gb = gb.with_token_info(tok, u64::MAX);
    }

    let group = match gb.build_blocking() {
        Ok(g)  => g,
        Err(e) => {
            let msg = format!("group build: {e}");
            error::set(&msg);
            return classify_error(&msg);
        }
    };

    // Queue files (each download starts immediately in the background).
    for (info, dest) in items {
        if let Err(e) = group.download_file_to_path_blocking(info, dest) {
            let msg = format!("enqueue: {e}");
            error::set(&msg);
            return classify_error(&msg);
        }
    }

    // Progress-polling thread. Cloning the group is cheap (Arc).
    let finished = Arc::new(AtomicBool::new(false));
    let progress_handle = if let Some(cb) = progress_cb {
        let g    = group.clone();
        let done = finished.clone();
        // user_data pointer crosses threads; the C caller owns
        // synchronization semantics of whatever it points to. We
        // wrap in a usize to satisfy Send.
        let ud_usize = user_data as usize;
        Some(thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let p = g.progress();
                cb(ud_usize as *mut c_void, p.total_bytes_completed, p.total_bytes);
                thread::sleep(Duration::from_millis(250));
            }
        }))
    } else {
        None
    };

    // Wait for all downloads to complete on the main thread.
    let result = group.finish_blocking();

    finished.store(true, Ordering::Release);
    if let Some(h) = progress_handle { let _ = h.join(); }

    match result {
        Ok(report) => {
            if let Some(cb) = progress_cb {
                cb(user_data, report.progress.total_bytes, report.progress.total_bytes);
            }
            LXET_OK
        }
        Err(e) => {
            let msg = format!("finish: {e}");
            error::set(&msg);
            classify_error(&msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_session_returns_invalid_arg() {
        let rc = download_inner(std::ptr::null_mut(), std::ptr::null(), 0, None, std::ptr::null_mut());
        assert_eq!(rc, LXET_ERR_INVALID_ARG);
    }

    #[test]
    fn zero_files_is_trivially_ok() {
        // Need a real session for this.
        let s = crate::session::new_inner(std::ptr::null(), std::ptr::null(), std::ptr::null()).unwrap();
        let raw = Box::into_raw(s);
        let rc = download_inner(raw, std::ptr::null(), 0, None, std::ptr::null_mut());
        assert_eq!(rc, LXET_OK);
        crate::session::free_inner(raw);
    }

    #[test]
    fn null_files_with_nonzero_count_is_invalid() {
        let s = crate::session::new_inner(std::ptr::null(), std::ptr::null(), std::ptr::null()).unwrap();
        let raw = Box::into_raw(s);
        let rc = download_inner(raw, std::ptr::null(), 3, None, std::ptr::null_mut());
        assert_eq!(rc, LXET_ERR_INVALID_ARG);
        crate::session::free_inner(raw);
    }

    #[test]
    fn classify_error_heuristics() {
        assert_eq!(classify_error("user cancelled"),           LXET_ERR_CANCELLED);
        assert_eq!(classify_error("http 401 unauthorized"),    LXET_ERR_AUTH);
        assert_eq!(classify_error("chunk integrity failed"),   LXET_ERR_INTEGRITY);
        assert_eq!(classify_error("connection timeout"),       LXET_ERR_NETWORK);
        assert_eq!(classify_error("something else entirely"),  LXET_ERR_OTHER);
    }
}
