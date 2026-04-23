//! Thread-local last-error storage for the llama-xet C ABI.
//!
//! C callers use `llama_xet_last_error()` to retrieve a diagnostic
//! string for the most recent failing FFI call on their thread.
//! Internal Rust code sets the string via `set()` before returning
//! a failure indicator (null pointer, nonzero code, etc.).
//!
//! Semantics match the C convention (libc errno, OpenSSL ERR_get_error):
//!   - Thread-local: concurrent threads do not see each other's errors.
//!   - Valid until overwritten: the next FFI call on the same thread
//!     may invalidate the pointer. Callers must copy the string if
//!     they need it beyond the next FFI invocation.
//!   - Empty-by-default: before any failure, returns a pointer to an
//!     empty C string (never a null pointer).

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Set the current thread's last-error message. Called from inside
/// FFI functions on any failure path.
///
/// Invalid UTF-8 input is replaced with a placeholder so we never
/// return a null pointer from `llama_xet_last_error`.
pub fn set<S: Into<String>>(msg: S) {
    let raw = msg.into();
    let c = CString::new(raw).unwrap_or_else(|_| {
        CString::new("(llama-xet: error message contained NUL)").unwrap()
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(c));
}

/// Clear the current thread's last-error. Called at the start of
/// every FFI function so a successful call does not leak a stale
/// error from an earlier failure on the same thread.
pub fn clear() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Returns a pointer to the current thread's last-error C string,
/// or a pointer to an empty static C string if no error is set.
///
/// The extern "C" entry point `llama_xet_last_error` lives in lib.rs
/// so cbindgen picks it up (cbindgen's submodule traversal is flaky
/// for `#[no_mangle]` items declared inside `mod foo;`).
pub fn get_ptr() -> *const c_char {
    LAST_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(b"\0".as_ptr() as *const c_char)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn initially_empty() {
        let ptr = get_ptr();
        assert!(!ptr.is_null());
        unsafe { assert_eq!(*ptr, 0) };
    }

    #[test]
    fn set_get_roundtrip() {
        set("download failed: 503");
        let ptr = get_ptr();
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "download failed: 503");
        clear();
    }

    #[test]
    fn clear_resets_to_empty() {
        set("stale");
        clear();
        let ptr = get_ptr();
        unsafe { assert_eq!(*ptr, 0) };
    }

    #[test]
    fn nul_in_message_is_handled() {
        set("bad\0message");
        let ptr = get_ptr();
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(s.contains("contained NUL"), "got: {s}");
        clear();
    }

    #[test]
    fn overwrite_replaces_previous() {
        set("first");
        set("second");
        let ptr = get_ptr();
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "second");
        clear();
    }
}
