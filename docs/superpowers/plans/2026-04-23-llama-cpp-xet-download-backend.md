# llama.cpp hf-xet Download Backend Implementation Plan

> **Important — AI policy note:** llama.cpp's `AGENTS.md` forbids AI-generated code in upstream PRs. This plan is a reading reference for a **human contributor** (Rajat Arya, `@rajatarya`) — not a script for agent execution. All code in this document is illustrative; the contributor rewrites every line in their own voice, understands it, and defends it in review without AI assistance. Commits and PR descriptions are human-authored. AI assistance during implementation must be disclosed in the PR body per `AGENTS.md`.

**Goal:** Add an optional `LLAMA_XET` build flag that routes Hugging Face model downloads through xet-core's `XetSession` (chunk-level dedup + resume), with silent fallback to the existing cpp-httplib path for non-Xet-backed files or Xet errors.

**Architecture:** A new in-tree Rust crate (`common/llama-xet/`) exposes a narrow C ABI over `xet_pkg::XetSession`. A guarded branch in `common_download_model()` invokes this ABI when all files in a plan are Xet-backed; otherwise the existing `std::async` + cpp-httplib fanout runs unchanged. Build-system pattern cloned from the existing `llguidance` integration.

**Tech Stack:** C++17 (llama.cpp), Rust 2021 (wrapper crate), CMake 3.x (`ExternalProject_Add` + `cargo build`), cbindgen (auto-generated C header), xet-core pinned at SHA `b43c0aec`.

**Reference spec:** `docs/superpowers/specs/2026-04-23-llama-cpp-xet-integration-design.md` — read the whole thing before Task 1. That document answers the "why" for every decision here.

---

## Preflight — Verify the HF API shape before writing code

The spec assumes the HF tree API returns a `xet_hash` field per file when the file is Xet-backed, and that `/api/models/{repo}/xet-read-token/{rev}` returns `{accessToken, exp, casUrl}`. **Both of these need live verification before coding begins.** If the schema differs, the `hf_cache` changes in Tasks 8–9 below change accordingly.

- [ ] **Step P1: Verify tree API `xet_hash` field** — pick a known Xet-backed repo (e.g., any recent `TheBloke/*-GGUF` or a Llama 3.x GGUF repo published with Xet enabled) and query:

  ```bash
  curl -H "Authorization: Bearer $HF_TOKEN" \
    "https://huggingface.co/api/models/<org>/<repo>/tree/main?recursive=true" \
    | jq '.[] | select(.path | endswith(".gguf"))'
  ```

  Expected: entries for Xet-backed files include a field naming the Xet Merkle hash. Record the exact field name in your notes — the plan uses `xet_hash` but the real name may be `xet-hash`, `xetHash`, nested under `lfs.xet`, etc. Adjust Task 8 accordingly.

- [ ] **Step P2: Verify xet-read-token endpoint** — hit the token endpoint on the same repo:

  ```bash
  curl -H "Authorization: Bearer $HF_TOKEN" \
    "https://huggingface.co/api/models/<org>/<repo>/xet-read-token/main"
  ```

  Expected: JSON with the scoped token, expiry, and CAS URL. Record the exact field names. Adjust Task 9's JSON parsing to match.

- [ ] **Step P3: Verify no surprises** — repeat on a **non-Xet repo** (older LFS-only GGUF). Expected: tree API returns files *without* the Xet field, and the token endpoint returns 404 or similar. This confirms llama.cpp can distinguish Xet from non-Xet repos by metadata alone.

- [ ] **Step P4: Pick one Xet-backed repo for integration testing** — small enough to download in CI (<100 MB recommended). Note the org/repo/filename — you'll reference it in Tasks 5, 14, 17.

No commit — this is scratch verification.

---

## Task 1: Add `LLAMA_XET` CMake option (no-op build)

**Files:**
- Modify: `CMakeLists.txt` (add option near line 117, next to `LLAMA_LLGUIDANCE`)
- Modify: `common/CMakeLists.txt` (add a guarded block near line 140, modeled after the `llguidance` block)
- Create: `common/llama-xet/Cargo.toml` (minimal, empty lib)
- Create: `common/llama-xet/src/lib.rs` (empty)

Goal: when the contributor runs `cmake -B build -DLLAMA_XET=ON && cmake --build build --target llama-common`, the build succeeds, produces `libllama_xet.a`, but nothing else changes. No C ABI yet.

- [ ] **Step 1.1: Add the top-level option**

  In `CMakeLists.txt` near the existing `option(LLAMA_LLGUIDANCE ...)` line (~117):

  ```cmake
  option(LLAMA_XET "llama-common: use hf-xet for HF model downloads" OFF)
  ```

- [ ] **Step 1.2: Add the `common/llama-xet/` skeleton**

  `common/llama-xet/Cargo.toml`:
  ```toml
  [package]
  name    = "llama-xet"
  version = "0.1.0"
  edition = "2021"

  [lib]
  name       = "llama_xet"
  crate-type = ["staticlib"]

  [dependencies]
  ```

  `common/llama-xet/src/lib.rs`:
  ```rust
  // Intentionally empty — populated in Task 4.
  ```

- [ ] **Step 1.3: Add the CMake block**

  In `common/CMakeLists.txt`, near the existing `if (LLAMA_LLGUIDANCE)` block (~line 140), add **after** the llguidance block (so the two are adjacent and obviously parallel):

  ```cmake
  if (LLAMA_XET)
      if (NOT LLAMA_OPENSSL)
          message(FATAL_ERROR
              "LLAMA_XET=ON requires LLAMA_OPENSSL=ON "
              "(HF metadata API is still fetched over HTTPS).")
      endif()

      include(ExternalProject)
      set(LLAMA_XET_SRC  ${CMAKE_SOURCE_DIR}/common/llama-xet)
      set(LLAMA_XET_PATH ${CMAKE_BINARY_DIR}/llama-xet/target/release)
      set(LLAMA_XET_LIB_NAME "${CMAKE_STATIC_LIBRARY_PREFIX}llama_xet${CMAKE_STATIC_LIBRARY_SUFFIX}")

      ExternalProject_Add(llama_xet_ext
          SOURCE_DIR         ${LLAMA_XET_SRC}
          PREFIX             ${CMAKE_BINARY_DIR}/llama-xet
          CONFIGURE_COMMAND  ""
          BUILD_COMMAND      cargo build --release --target-dir ${CMAKE_BINARY_DIR}/llama-xet/target
          BUILD_IN_SOURCE    1
          INSTALL_COMMAND    ""
          BUILD_BYPRODUCTS   ${LLAMA_XET_PATH}/${LLAMA_XET_LIB_NAME}
      )

      target_compile_definitions(${TARGET} PUBLIC LLAMA_USE_XET)
      add_library(llama_xet STATIC IMPORTED)
      set_target_properties(llama_xet PROPERTIES IMPORTED_LOCATION ${LLAMA_XET_PATH}/${LLAMA_XET_LIB_NAME})
      add_dependencies(llama_xet llama_xet_ext)
      target_link_libraries(${TARGET} PRIVATE llama_xet)
  endif()
  ```

  Note: `BUILD_BYPRODUCTS` intentionally does NOT list a header file yet — we add cbindgen in Task 3.

- [ ] **Step 1.4: Verify OFF build is unchanged**

  ```bash
  rm -rf build && cmake -B build && cmake --build build --target llama-common -j
  ```
  Expected: clean build, no mention of llama_xet.

- [ ] **Step 1.5: Verify ON build succeeds**

  ```bash
  rm -rf build && cmake -B build -DLLAMA_XET=ON && cmake --build build --target llama-common -j
  ```
  Expected: cargo compiles the empty crate, `libllama_xet.a` exists under `build/llama-xet/target/release/`, `llama-common` links successfully. No new runtime behavior.

- [ ] **Step 1.6: Verify the OPENSSL guard works**

  ```bash
  rm -rf build && cmake -B build -DLLAMA_XET=ON -DLLAMA_OPENSSL=OFF
  ```
  Expected: `FATAL_ERROR` from CMake.

- [ ] **Step 1.7: Commit**

  ```
  common : add LLAMA_XET build flag and empty wrapper crate (#<issue>)
  ```

  Files: `CMakeLists.txt`, `common/CMakeLists.txt`, `common/llama-xet/Cargo.toml`, `common/llama-xet/src/lib.rs`.

---

## Task 2: Pin xet-core and verify it compiles

**Files:**
- Modify: `common/llama-xet/Cargo.toml`

Goal: add `xet_pkg` as a dependency at SHA `b43c0aec`. Verify it compiles (this may take several minutes the first time — tokio + reqwest + blake3 is a big tree).

- [ ] **Step 2.1: Add the dependency**

  ```toml
  [dependencies]
  xet_pkg = { git = "https://github.com/huggingface/xet-core", rev = "b43c0aec" }
  libc    = "0.2"
  ```

- [ ] **Step 2.2: Add a trivial probe that references XetSession**

  In `common/llama-xet/src/lib.rs`:
  ```rust
  // Temporary — deleted in Task 4. Proves xet_pkg links.
  #[allow(dead_code)]
  fn _probe_xet_session_exists() -> Option<&'static str> {
      // Reference the type so the linker keeps it.
      std::any::type_name::<xet_pkg::xet_session::XetSession>().into()
  }
  ```

  Note: the module path is `xet_pkg::xet_session::XetSession`. If the path differs in the pinned SHA, fix the use statement — check `xet_pkg/src/lib.rs` in the xet-core checkout under `/Users/rajat/code/hf/xet-core`.

- [ ] **Step 2.3: Build**

  ```bash
  cmake --build build --target llama-common -j
  ```
  Expected: first build downloads xet-core deps (multi-minute). Subsequent builds are incremental. `libllama_xet.a` is now ~several MB.

- [ ] **Step 2.4: Commit**

  ```
  common : pin xet-core dependency in llama-xet crate (#<issue>)
  ```

---

## Task 3: Wire cbindgen for generated C header

**Files:**
- Modify: `common/llama-xet/Cargo.toml` (add build-dependency)
- Create: `common/llama-xet/build.rs`
- Create: `common/llama-xet/cbindgen.toml`
- Modify: `common/CMakeLists.txt` (add generated header to `BUILD_BYPRODUCTS` and to include paths)

Goal: `cargo build` produces `target/release/llama-xet.h` automatically. CMake's `ExternalProject_Add` expects it as a byproduct and propagates the path to C++ includes.

- [ ] **Step 3.1: Add cbindgen as build-dep**

  ```toml
  [build-dependencies]
  cbindgen = "0.27"
  ```

- [ ] **Step 3.2: Write `build.rs`**

  `common/llama-xet/build.rs`:
  ```rust
  use std::env;
  use std::path::PathBuf;

  fn main() {
      let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
      // Place header next to libllama_xet.a so CMake finds both in BUILD_BYPRODUCTS.
      let out_dir = env::var("CARGO_TARGET_DIR")
          .or_else(|_| env::var("OUT_DIR"))
          .map(PathBuf::from)
          .unwrap_or_else(|_| PathBuf::from(&crate_dir).join("target"));
      let header_path = out_dir.join("release").join("llama-xet.h");

      cbindgen::Builder::new()
          .with_crate(&crate_dir)
          .with_config(cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).unwrap())
          .generate()
          .expect("cbindgen failed")
          .write_to_file(&header_path);

      println!("cargo:rerun-if-changed=src/lib.rs");
      println!("cargo:rerun-if-changed=cbindgen.toml");
  }
  ```

- [ ] **Step 3.3: Write `cbindgen.toml`**

  ```toml
  language = "C"
  header = "/* Auto-generated by cbindgen — do not edit. */"
  include_guard = "LLAMA_XET_H"
  autogen_warning = "/* Auto-generated header. */"
  style = "type"
  cpp_compat = true
  documentation = true

  [export]
  prefix = "LlamaXet"

  [parse]
  parse_deps = false
  ```

- [ ] **Step 3.4: Add one `#[no_mangle]` probe export so cbindgen has something to emit**

  In `src/lib.rs`, replace the probe:
  ```rust
  /// Returns the pinned xet-core git revision at build time.
  /// Callers should NOT free the returned pointer.
  #[no_mangle]
  pub extern "C" fn llama_xet_version(void *) -> *const core::ffi::c_char {
      concat!("xet-core ", "b43c0aec", "\0").as_ptr() as *const _
  }
  ```

  (Note: `extern "C"` fn, no mangling, returns a static C string. Adjust the signature — this is just to prove the toolchain wires up.)

- [ ] **Step 3.5: Update CMake to know about the generated header**

  In `common/CMakeLists.txt` `LLAMA_XET` block, update `BUILD_BYPRODUCTS` and add `target_include_directories`:

  ```cmake
      BUILD_BYPRODUCTS   ${LLAMA_XET_PATH}/${LLAMA_XET_LIB_NAME}
                         ${LLAMA_XET_PATH}/llama-xet.h
  )

  target_compile_definitions(${TARGET} PUBLIC LLAMA_USE_XET)
  add_library(llama_xet STATIC IMPORTED)
  set_target_properties(llama_xet PROPERTIES IMPORTED_LOCATION ${LLAMA_XET_PATH}/${LLAMA_XET_LIB_NAME})
  add_dependencies(llama_xet llama_xet_ext)
  target_include_directories(${TARGET} PRIVATE ${LLAMA_XET_PATH})
  target_link_libraries(${TARGET} PRIVATE llama_xet)
  ```

- [ ] **Step 3.6: Build and verify the header is produced**

  ```bash
  cmake --build build --target llama-common -j
  cat build/llama-xet/target/release/llama-xet.h
  ```
  Expected: header exists, contains `llama_xet_version` declaration.

- [ ] **Step 3.7: Commit**

  ```
  common : generate llama-xet.h via cbindgen (#<issue>)
  ```

---

## Task 4: Thread-local error storage

**Files:**
- Create: `common/llama-xet/src/error.rs`
- Modify: `common/llama-xet/src/lib.rs`

Goal: implement `llama_xet_last_error()` returning a thread-local C string. All subsequent FFI functions write their error messages here on failure.

- [ ] **Step 4.1: Error module**

  `common/llama-xet/src/error.rs`:
  ```rust
  use std::cell::RefCell;
  use std::ffi::CString;
  use std::os::raw::c_char;

  thread_local! {
      static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
  }

  pub fn set<S: Into<String>>(msg: S) {
      let c = CString::new(msg.into()).unwrap_or_else(|_| CString::new("(invalid utf-8)").unwrap());
      LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(c));
  }

  pub fn clear() {
      LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
  }

  /// Returns a pointer to the current thread's last error message,
  /// or an empty static string if no error is set.
  ///
  /// # Safety
  /// The returned pointer is valid until the next FFI call on this
  /// thread. Do not free it.
  #[no_mangle]
  pub extern "C" fn llama_xet_last_error() -> *const c_char {
      LAST_ERROR.with(|slot| {
          slot.borrow()
              .as_ref()
              .map(|c| c.as_ptr())
              .unwrap_or(b"\0".as_ptr() as *const c_char)
      })
  }
  ```

- [ ] **Step 4.2: Wire into lib.rs**

  `common/llama-xet/src/lib.rs`:
  ```rust
  mod error;

  pub use error::llama_xet_last_error;
  ```

- [ ] **Step 4.3: Test**

  Add `common/llama-xet/src/tests.rs` or an inline `#[cfg(test)] mod tests` block:
  ```rust
  #[test]
  fn last_error_initially_empty() {
      unsafe {
          let ptr = error::llama_xet_last_error();
          assert_eq!(*ptr, 0, "initial last_error should be empty C string");
      }
  }

  #[test]
  fn last_error_roundtrip() {
      error::set("boom");
      unsafe {
          let ptr = error::llama_xet_last_error();
          let s = std::ffi::CStr::from_ptr(ptr).to_str().unwrap();
          assert_eq!(s, "boom");
      }
      error::clear();
  }
  ```

  Run: `cargo test --manifest-path common/llama-xet/Cargo.toml`
  Expected: both tests pass.

- [ ] **Step 4.4: Commit**

  ```
  common : add thread-local last-error machinery for llama-xet FFI (#<issue>)
  ```

---

## Task 5: Session lifecycle — `new`, `free`, `abort`

**Files:**
- Create: `common/llama-xet/src/session.rs`
- Modify: `common/llama-xet/src/lib.rs`

Goal: expose `llama_xet_session_new`, `llama_xet_session_free`, `llama_xet_session_abort`. Backed by `XetSessionBuilder::build()` and `XetSession::sigint_abort()`.

- [ ] **Step 5.1: Session wrapper**

  `common/llama-xet/src/session.rs`:
  ```rust
  use std::ffi::CStr;
  use std::os::raw::c_char;
  use xet_pkg::xet_session::{XetSession, XetSessionBuilder};

  use crate::error;

  /// Opaque handle. C sees `struct LlamaXetSession;` — never dereferenced.
  pub struct LlamaXetSession {
      inner:              XetSession,
      endpoint:           Option<String>,
      bearer_token:       Option<String>,
      token_refresh_url:  Option<String>,
  }

  fn cstr_to_opt_string(p: *const c_char) -> Option<String> {
      if p.is_null() {
          return None;
      }
      unsafe { CStr::from_ptr(p) }.to_str().ok().map(str::to_owned)
  }

  #[no_mangle]
  pub extern "C" fn llama_xet_session_new(
      endpoint:          *const c_char,
      bearer_token:      *const c_char,
      token_refresh_url: *const c_char,
  ) -> *mut LlamaXetSession {
      error::clear();
      let build = || -> Result<LlamaXetSession, String> {
          let endpoint          = cstr_to_opt_string(endpoint);
          let bearer_token      = cstr_to_opt_string(bearer_token);
          let token_refresh_url = cstr_to_opt_string(token_refresh_url);

          let session = XetSessionBuilder::new()
              .build()
              .map_err(|e| format!("XetSessionBuilder::build failed: {e}"))?;

          Ok(LlamaXetSession { inner: session, endpoint, bearer_token, token_refresh_url })
      };

      match build() {
          Ok(s)  => Box::into_raw(Box::new(s)),
          Err(e) => {
              error::set(e);
              std::ptr::null_mut()
          }
      }
  }

  #[no_mangle]
  pub extern "C" fn llama_xet_session_free(session: *mut LlamaXetSession) {
      if session.is_null() {
          return;
      }
      unsafe { drop(Box::from_raw(session)); }
  }

  #[no_mangle]
  pub extern "C" fn llama_xet_session_abort(session: *mut LlamaXetSession) {
      if session.is_null() {
          return;
      }
      // Safe: pointer comes from Box::into_raw above.
      let s = unsafe { &*session };
      let _ = s.inner.sigint_abort();
  }
  ```

- [ ] **Step 5.2: Wire into lib.rs**

  ```rust
  mod error;
  mod session;

  pub use error::llama_xet_last_error;
  pub use session::{LlamaXetSession, llama_xet_session_new, llama_xet_session_free, llama_xet_session_abort};
  ```

- [ ] **Step 5.3: Test**

  ```rust
  #[test]
  fn session_new_free_roundtrip() {
      let s = super::session::llama_xet_session_new(
          std::ptr::null(), std::ptr::null(), std::ptr::null());
      assert!(!s.is_null(), "session_new should succeed with NULLs");
      super::session::llama_xet_session_free(s);
  }

  #[test]
  fn session_free_null_is_safe() {
      super::session::llama_xet_session_free(std::ptr::null_mut());
  }

  #[test]
  fn session_abort_null_is_safe() {
      super::session::llama_xet_session_abort(std::ptr::null_mut());
  }
  ```

  Run: `cargo test --manifest-path common/llama-xet/Cargo.toml`
  Expected: three tests pass; no sanitizer complaints if run under `cargo +nightly miri test`.

- [ ] **Step 5.4: Verify generated header**

  ```bash
  cmake --build build --target llama-common -j
  grep -E 'llama_xet_(session|last_error)' build/llama-xet/target/release/llama-xet.h
  ```
  Expected: all four symbols appear in the generated header.

- [ ] **Step 5.5: Commit**

  ```
  common : add XetSession lifecycle FFI (new/free/abort) (#<issue>)
  ```

---

## Task 6: Download entry point — `llama_xet_download_files`

**Files:**
- Create: `common/llama-xet/src/download.rs`
- Modify: `common/llama-xet/src/lib.rs`

Goal: implement the batch download call using `XetSession::new_file_download_group()` → `XetFileDownloadGroup::download_file_to_path()` → `finish_blocking()`.

Read `xet_pkg/src/xet_session/file_download_group.rs` before writing this — in particular `XetFileDownloadGroupBuilder::build()`, `download_file_to_path()`, and `finish_blocking()`. Also read the Python consumer `hf_xet/src/lib.rs:274` to confirm parameter order and error shapes.

- [ ] **Step 6.1: Public struct & function**

  `common/llama-xet/src/download.rs`:
  ```rust
  use std::ffi::CStr;
  use std::os::raw::{c_char, c_void};
  use std::path::PathBuf;

  use xet_pkg::xet_session::XetFileInfo;

  use crate::error;
  use crate::session::LlamaXetSession;

  #[repr(C)]
  pub struct LlamaXetFileInfo {
      pub hash:      *const c_char,
      pub file_size: u64,
      pub dest_path: *const c_char,
  }

  pub type LlamaXetProgressFn = Option<
      extern "C" fn(user_data: *mut c_void, completed: u64, total: u64)
  >;

  /// Errors returned by llama_xet_download_files.
  /// Keep in sync with common/xet.h error-code constants.
  pub const LXET_OK:              i32 = 0;
  pub const LXET_ERR_INVALID_ARG: i32 = 1;
  pub const LXET_ERR_AUTH:        i32 = 2;
  pub const LXET_ERR_NETWORK:     i32 = 3;
  pub const LXET_ERR_INTEGRITY:   i32 = 4;
  pub const LXET_ERR_CANCELLED:   i32 = 5;
  pub const LXET_ERR_OTHER:       i32 = 99;

  #[no_mangle]
  pub extern "C" fn llama_xet_download_files(
      session:     *mut LlamaXetSession,
      files:       *const LlamaXetFileInfo,
      file_count:  usize,
      progress_cb: LlamaXetProgressFn,
      user_data:   *mut c_void,
  ) -> i32 {
      error::clear();

      if session.is_null() || (files.is_null() && file_count > 0) {
          error::set("null session or null files array");
          return LXET_ERR_INVALID_ARG;
      }

      let session = unsafe { &*session };
      let slice   = unsafe { std::slice::from_raw_parts(files, file_count) };

      // Marshal C descriptors → XetFileInfo + dest_path tuples.
      let mut items: Vec<(XetFileInfo, PathBuf)> = Vec::with_capacity(file_count);
      for (i, f) in slice.iter().enumerate() {
          let hash = match unsafe { CStr::from_ptr(f.hash) }.to_str() {
              Ok(s) => s.to_owned(),
              Err(_) => { error::set(format!("file[{i}].hash is not valid UTF-8")); return LXET_ERR_INVALID_ARG; }
          };
          let dest = match unsafe { CStr::from_ptr(f.dest_path) }.to_str() {
              Ok(s) => PathBuf::from(s),
              Err(_) => { error::set(format!("file[{i}].dest_path is not valid UTF-8")); return LXET_ERR_INVALID_ARG; }
          };
          let info = XetFileInfo {
              hash,
              file_size: if f.file_size == 0 { None } else { Some(f.file_size) },
              sha256: None,
          };
          items.push((info, dest));
      }

      // Build the download group. Wire token + refresh URL + endpoint.
      let mut gb = match session.inner.new_file_download_group() {
          Ok(gb) => gb,
          Err(e) => { error::set(format!("new_file_download_group: {e}")); return LXET_ERR_OTHER; }
      };

      if let Some(url) = session.token_refresh_url.as_deref() {
          gb = gb.with_token_refresh_url(url.to_owned(), Default::default());
      } else if let Some(tok) = session.bearer_token.as_deref() {
          // Static token with far-future expiry — xet-core will re-auth via HF if it expires.
          gb = gb.with_token_info(tok.to_owned(), u64::MAX);
      }
      if let Some(ep) = session.endpoint.as_deref() {
          gb = gb.with_endpoint(ep.to_owned());
      }

      let group = match gb.build() {
          Ok(g)  => g,
          Err(e) => { error::set(format!("group build: {e}")); return LXET_ERR_OTHER; }
      };

      // Queue files.
      let mut handles = Vec::with_capacity(items.len());
      for (info, dest) in items {
          match group.download_file_to_path(info, dest) {
              Ok(h)  => handles.push(h),
              Err(e) => { error::set(format!("enqueue: {e}")); return LXET_ERR_OTHER; }
          }
      }

      // Progress polling on a background thread until finish returns.
      let finish = std::thread::spawn({
          let group = group.clone();
          move || group.finish_blocking()
      });

      if let Some(cb) = progress_cb {
          loop {
              if finish.is_finished() { break; }
              let p = group.progress();
              cb(user_data, p.completed_bytes, p.total_bytes);
              std::thread::sleep(std::time::Duration::from_millis(250));
          }
      }

      let report = match finish.join() {
          Ok(Ok(r))  => r,
          Ok(Err(e)) => { error::set(format!("finish: {e}")); return classify(&e); }
          Err(_)     => { error::set("finish thread panicked"); return LXET_ERR_OTHER; }
      };

      // Final progress callback so caller sees 100%.
      if let Some(cb) = progress_cb {
          cb(user_data, report.total_bytes, report.total_bytes);
      }

      LXET_OK
  }

  fn classify(err: &dyn std::fmt::Display) -> i32 {
      // Cheap string-matching triage. Refine once we've seen real xet-core errors.
      let s = err.to_string().to_lowercase();
      if s.contains("cancel")     { return LXET_ERR_CANCELLED; }
      if s.contains("auth") || s.contains("401") || s.contains("403") { return LXET_ERR_AUTH; }
      if s.contains("integrity") || s.contains("hash mismatch")       { return LXET_ERR_INTEGRITY; }
      if s.contains("timeout") || s.contains("connect") || s.contains("network") { return LXET_ERR_NETWORK; }
      LXET_ERR_OTHER
  }
  ```

  **Note:** the exact shape of `XetFileInfo`, group-builder setters, and progress struct may not match verbatim — check `xet_pkg/src/xet_session/file_download_group.rs` and adjust. This function is where most implementation time is spent.

- [ ] **Step 6.2: Wire into lib.rs**

  ```rust
  mod download;
  pub use download::{LlamaXetFileInfo, LlamaXetProgressFn, llama_xet_download_files};
  ```

- [ ] **Step 6.3: Local smoke test**

  Add an integration test that uses the Xet-backed test file you chose in step P4. Gate it behind an env var so `cargo test` stays offline by default:

  `common/llama-xet/tests/download_smoke.rs`:
  ```rust
  use std::ffi::CString;

  #[test]
  #[ignore] // only when LLAMA_XET_E2E=1
  fn download_one_file() {
      if std::env::var("LLAMA_XET_E2E").is_err() { return; }

      // Fill in from step P4.
      let endpoint = std::env::var("LXET_ENDPOINT").unwrap();
      let token    = std::env::var("HF_TOKEN").unwrap();
      let hash     = std::env::var("LXET_HASH").unwrap();
      let dest     = tempfile::NamedTempFile::new().unwrap();

      let e_c = CString::new(endpoint).unwrap();
      let t_c = CString::new(token).unwrap();
      let h_c = CString::new(hash).unwrap();
      let d_c = CString::new(dest.path().to_str().unwrap()).unwrap();

      let session = llama_xet::llama_xet_session_new(
          e_c.as_ptr(), t_c.as_ptr(), std::ptr::null());
      assert!(!session.is_null());

      let info = llama_xet::LlamaXetFileInfo {
          hash: h_c.as_ptr(),
          file_size: 0,
          dest_path: d_c.as_ptr(),
      };
      let rc = llama_xet::llama_xet_download_files(
          session, &info, 1, None, std::ptr::null_mut());
      llama_xet::llama_xet_session_free(session);

      assert_eq!(rc, 0, "download should succeed");
      assert!(dest.path().metadata().unwrap().len() > 0);
  }
  ```

  Add `tempfile` as a dev-dependency. Run:
  ```bash
  LLAMA_XET_E2E=1 LXET_ENDPOINT=... HF_TOKEN=... LXET_HASH=... \
    cargo test --manifest-path common/llama-xet/Cargo.toml -- --ignored download_one_file
  ```
  Expected: the file downloads, test passes, content matches expected size.

- [ ] **Step 6.4: Commit**

  ```
  common : add llama_xet_download_files batch download FFI (#<issue>)
  ```

---

## Task 7: C++ RAII header — `common/xet.h`

**Files:**
- Create: `common/xet.h`

Goal: C++ callers get RAII and `std::string` ergonomics on top of the raw C ABI.

- [ ] **Step 7.1: Header**

  `common/xet.h`:
  ```cpp
  #pragma once

  // This header is a no-op unless LLAMA_USE_XET is defined (set by CMake
  // when LLAMA_XET=ON). Callers should guard includes with the same macro.

  #ifdef LLAMA_USE_XET

  #include "llama-xet.h"   // cbindgen-generated, in the llama-xet target/release dir

  #include <memory>
  #include <string>

  namespace llama_xet {

  struct session_deleter {
      void operator()(::LlamaXetSession * s) const noexcept {
          if (s) ::llama_xet_session_free(s);
      }
  };

  using session_ptr = std::unique_ptr<::LlamaXetSession, session_deleter>;

  inline session_ptr new_session(const char * endpoint,
                                 const char * bearer_token,
                                 const char * token_refresh_url) {
      return session_ptr(::llama_xet_session_new(endpoint, bearer_token, token_refresh_url));
  }

  inline std::string last_error() {
      const char * msg = ::llama_xet_last_error();
      return msg ? std::string(msg) : std::string();
  }

  } // namespace llama_xet

  #endif // LLAMA_USE_XET
  ```

- [ ] **Step 7.2: Add to `common/CMakeLists.txt` sources**

  Near line 84 where `llguidance.cpp` is listed, add `xet.h` (and Task 10's `xet.cpp`, created later). Sources are listed without `#ifdef` guards — the files themselves use `#ifdef LLAMA_USE_XET` to no-op when off.

- [ ] **Step 7.3: Verify it compiles**

  ```bash
  rm -rf build && cmake -B build -DLLAMA_XET=ON && cmake --build build --target llama-common -j
  ```
  Expected: no errors; `xet.h` is picked up.

- [ ] **Step 7.4: Commit**

  ```
  common : add C++ RAII wrapper around llama-xet FFI (xet.h) (#<issue>)
  ```

---

## Task 8: HF tree API — parse `xet_hash` field

**Files:**
- Modify: `common/hf-cache.h` (add `xet_hash` to `hf_file`)
- Modify: `common/hf-cache.cpp` (parse from tree response)
- Create: `tests/test-hf-cache-xet.cpp` (new C++ unit test using a canned JSON fixture)
- Create: `tests/fixtures/hf-tree-xet.json` (captured from step P1)

Goal: when the HF tree API returns a file with a Xet hash, it lands on `hf_file::xet_hash`. Non-Xet files get an empty string.

- [ ] **Step 8.1: Extend struct**

  In `common/hf-cache.h`, inside `struct hf_file`:
  ```cpp
  std::string xet_hash;   // empty if file is not Xet-backed
  uint64_t    xet_size = 0;
  ```

- [ ] **Step 8.2: Parse in tree response**

  In `common/hf-cache.cpp:get_repo_files()` (around line 312 where the tree response is walked), add field extraction matching whatever name you verified in step P1. Example (rename the JSON key if your P1 output differs):

  ```cpp
  if (entry.contains("xet_hash") && entry["xet_hash"].is_string()) {
      file.xet_hash = entry["xet_hash"].get<std::string>();
      if (entry.contains("size") && entry["size"].is_number_unsigned()) {
          file.xet_size = entry["size"].get<uint64_t>();
      }
  }
  ```

- [ ] **Step 8.3: Capture fixture**

  Re-run the curl from step P1, saving to `tests/fixtures/hf-tree-xet.json`. Also capture a non-Xet response as `tests/fixtures/hf-tree-nonxet.json`.

- [ ] **Step 8.4: Write failing test**

  `tests/test-hf-cache-xet.cpp`:
  ```cpp
  #include "hf-cache.h"
  #include <cassert>
  #include <fstream>
  #include <sstream>

  static std::string slurp(const char * path) {
      std::ifstream f(path);
      std::stringstream ss; ss << f.rdbuf();
      return ss.str();
  }

  int main() {
      // Direct-test the JSON parser — expose it via an internal header or
      // an in-file extern declaration for testing. Rajat: decide whether
      // to make the parser a free function in hf-cache.cpp (simpler) or
      // to factor it out (cleaner). The simpler path is fine for Task 8.

      auto json = slurp("tests/fixtures/hf-tree-xet.json");
      auto files = hf_cache::parse_tree_response(json);

      bool found_xet = false;
      for (const auto & f : files) {
          if (f.path.find(".gguf") != std::string::npos) {
              assert(!f.xet_hash.empty() && "gguf file should have xet_hash");
              found_xet = true;
          }
      }
      assert(found_xet);

      // Same test for the non-xet fixture must produce empty xet_hash.
      auto json2 = slurp("tests/fixtures/hf-tree-nonxet.json");
      auto files2 = hf_cache::parse_tree_response(json2);
      for (const auto & f : files2) {
          assert(f.xet_hash.empty() && "legacy file should not have xet_hash");
      }

      return 0;
  }
  ```

  Add to `tests/CMakeLists.txt` following the pattern of other test-* files there. Build and run:
  ```bash
  cmake --build build --target test-hf-cache-xet -j
  ./build/bin/test-hf-cache-xet
  ```
  Expected: passes.

- [ ] **Step 8.5: Commit**

  ```
  common : parse xet_hash from HF tree API response (#<issue>)
  ```

---

## Task 9: HF `xet-read-token` endpoint fetch

**Files:**
- Modify: `common/hf-cache.h` (add `struct hf_xet_token` and `get_xet_token()` declaration)
- Modify: `common/hf-cache.cpp` (implementation)
- Create: `tests/fixtures/hf-xet-token.json`
- Modify: `tests/test-hf-cache-xet.cpp` (add parser test)

Goal: llama.cpp can fetch a scoped Xet token for a given repo + revision. The raw HTTP call uses the existing cpp-httplib client (same as the tree call), so no new HTTP infra.

- [ ] **Step 9.1: Struct + function declaration**

  In `common/hf-cache.h`:
  ```cpp
  namespace hf_cache {

  struct hf_xet_token {
      std::string access_token;   // empty on failure
      uint64_t    expiry_unix_secs = 0;
      std::string cas_url;
  };

  // Fetches /api/models/{repo}/xet-read-token/{rev}.
  // On failure (non-Xet repo, 404, network), returns a token with empty fields.
  // Caller should check .access_token.empty().
  hf_xet_token get_xet_token(
      const std::string & repo,
      const std::string & rev,
      const std::string & bearer_token);

  } // namespace hf_cache
  ```

- [ ] **Step 9.2: Implementation**

  In `common/hf-cache.cpp`, next to `get_repo_files()`:
  ```cpp
  hf_xet_token get_xet_token(
      const std::string & repo,
      const std::string & rev,
      const std::string & bearer_token)
  {
      hf_xet_token out;

      auto client = common_http_client(std::string(MODEL_ENDPOINT_DEFAULT));
      if (!bearer_token.empty()) {
          client.set_default_headers({{"Authorization", "Bearer " + bearer_token}});
      }

      const std::string path = "/api/models/" + repo + "/xet-read-token/" + rev;
      auto res = client.Get(path.c_str());
      if (!res || res->status != 200) {
          // Caller will check access_token.empty() and fall back.
          return out;
      }

      try {
          auto j = nlohmann::json::parse(res->body);
          // Adjust field names based on P2 verification.
          if (j.contains("accessToken")) out.access_token = j["accessToken"].get<std::string>();
          if (j.contains("exp"))         out.expiry_unix_secs = j["exp"].get<uint64_t>();
          if (j.contains("casUrl"))      out.cas_url = j["casUrl"].get<std::string>();
      } catch (const std::exception &) {
          out = {};
      }
      return out;
  }
  ```

- [ ] **Step 9.3: Add parser test (offline)**

  Factor the JSON-parsing portion into a `parse_xet_token_response(const std::string & body)` helper so it's testable without HTTP. Save a real token response (redact the actual token value!) to `tests/fixtures/hf-xet-token.json`, then:

  ```cpp
  auto tok = hf_cache::parse_xet_token_response(slurp("tests/fixtures/hf-xet-token.json"));
  assert(!tok.cas_url.empty());
  ```

- [ ] **Step 9.4: Commit**

  ```
  common : fetch Xet read-token from HF API (#<issue>)
  ```

---

## Task 10: `try_xet_download()` in `common/xet.cpp`

**Files:**
- Create: `common/xet.cpp`

Goal: the C++-side function the orchestrator calls. Marshals an `hf_plan` into `LlamaXetFileInfo[]`, creates a session, runs the batch download, translates errors.

- [ ] **Step 10.1: Sketch**

  ```cpp
  #include "xet.h"
  #include "hf-cache.h"
  #include "download.h"     // for common_download_opts, progress types
  #include "log.h"

  #ifdef LLAMA_USE_XET

  namespace llama_xet {

  struct try_result {
      bool        ok = false;
      std::string error;
  };

  try_result try_xet_download(
      const struct hf_plan &                plan,
      const struct common_download_opts &   opts,
      const hf_cache::hf_xet_token &        token)
  {
      try_result r;

      if (token.access_token.empty() || token.cas_url.empty()) {
          r.error = "no xet read-token available";
          return r;
      }

      const std::string refresh_url =
          std::string(opts.endpoint_or_default()) + "/api/models/" +
          plan.repo_id + "/xet-read-token/" + plan.revision;

      auto session = new_session(
          token.cas_url.c_str(),
          token.access_token.c_str(),
          refresh_url.c_str());
      if (!session) {
          r.error = "session_new failed: " + last_error();
          return r;
      }

      // Marshal files. Keep strings alive for the duration of the call.
      std::vector<std::string>         hashes, paths;
      std::vector<LlamaXetFileInfo>    infos;
      infos.reserve(plan.model_files.size() + (plan.mmproj.path.empty() ? 0 : 1));

      auto push = [&](const hf_cache::hf_file & f) {
          hashes.push_back(f.xet_hash);
          paths.push_back(f.local_path + ".downloadInProgress");
          LlamaXetFileInfo i{};
          i.hash      = hashes.back().c_str();
          i.file_size = f.xet_size;
          i.dest_path = paths.back().c_str();
          infos.push_back(i);
      };
      for (const auto & f : plan.model_files) push(f);
      if (!plan.mmproj.path.empty()) push(plan.mmproj);

      int rc = llama_xet_download_files(
          session.get(), infos.data(), infos.size(),
          nullptr, nullptr);   // Task 11 wires in a progress callback.
      if (rc != 0) {
          r.error = "download_files rc=" + std::to_string(rc) + ": " + last_error();
          return r;
      }

      // Move .downloadInProgress → final name (mirrors existing atomic-rename semantics).
      for (size_t i = 0; i < paths.size(); ++i) {
          std::error_code ec;
          const auto & dst = (i < plan.model_files.size())
              ? plan.model_files[i].local_path
              : plan.mmproj.local_path;
          std::filesystem::rename(paths[i], dst, ec);
          if (ec) {
              r.error = "rename: " + ec.message();
              return r;
          }
      }

      r.ok = true;
      return r;
  }

  } // namespace llama_xet

  #endif // LLAMA_USE_XET
  ```

  **Note:** `hf_plan` currently doesn't carry `repo_id` or `revision` — add those fields in Task 9's struct change, or thread them through from `common_download_model` separately. Tiny additional delta.

- [ ] **Step 10.2: Commit**

  ```
  common : implement try_xet_download() bridge (#<issue>)
  ```

---

## Task 11: Progress callback bridge

**Files:**
- Modify: `common/xet.cpp` (wire progress through the existing `ProgressBar` or callback type)

Goal: the progress callback from `common_download_opts` gets invoked during the Xet download, so the CLI progress bar works the same as the cpp-httplib path.

- [ ] **Step 11.1: Define a thunk**

  Inside `try_xet_download`:
  ```cpp
  struct progress_ctx {
      const common_download_opts * opts;
      uint64_t last_percent;
  };
  progress_ctx ctx{ &opts, 0 };

  auto thunk = [](void * ud, uint64_t done, uint64_t total) {
      auto * c = static_cast<progress_ctx *>(ud);
      if (!c->opts->callback || total == 0) return;
      uint64_t pct = (done * 1000) / total;   // 0.1% granularity
      if (pct != c->last_percent) {
          c->last_percent = pct;
          c->opts->callback(c->opts->callback_user_data, done, total);
      }
  };

  int rc = llama_xet_download_files(
      session.get(), infos.data(), infos.size(),
      thunk, &ctx);
  ```

- [ ] **Step 11.2: Verify manually**

  Build with `LLAMA_XET=ON`, run `llama-cli -hf <xet-backed repo>`, confirm progress updates appear at 0.1% intervals. No automated test — this is observational.

- [ ] **Step 11.3: Commit**

  ```
  common : wire progress callback through xet download path (#<issue>)
  ```

---

## Task 12: Orchestrator branch in `common_download_model`

**Files:**
- Modify: `common/download.cpp` (around line 690, after `get_hf_plan()`)
- Modify: `common/hf-cache.h` (add `all_files_xet_backed()` helper, extend `hf_plan` with `repo_id` + `revision` if not already done)

Goal: when all model files are Xet-backed, divert to `try_xet_download`; on any failure, fall through to existing fanout.

- [ ] **Step 12.1: Helper**

  In `common/hf-cache.h`:
  ```cpp
  inline bool all_files_xet_backed(const hf_files & files) {
      if (files.empty()) return false;
      for (const auto & f : files) {
          if (f.xet_hash.empty()) return false;
      }
      return true;
  }
  ```

- [ ] **Step 12.2: Fetch token in get_hf_plan**

  In `common/download.cpp`, extend `get_hf_plan` to also populate a new `hf_plan::xet_token` field via `hf_cache::get_xet_token(repo, commit, opts.bearer_token)` **only when** the tree response showed at least one xet_hash. If no Xet, skip the HTTP call entirely.

- [ ] **Step 12.3: Add the branch**

  In `common_download_model`, after the `get_hf_plan(...)` call and before the `std::async` fanout:
  ```cpp
  #ifdef LLAMA_USE_XET
      if (is_hf && !opts.offline && all_files_xet_backed(hf.model_files)) {
          auto result = llama_xet::try_xet_download(hf, opts, hf.xet_token);
          if (result.ok) {
              for (const auto & f : hf.model_files) hf_cache::finalize_file(f);
              common_download_model_result r;
              r.model_path  = hf.primary.final_path;
              if (!hf.mmproj.path.empty()) {
                  r.mmproj_path = hf_cache::finalize_file(hf.mmproj);
              }
              return r;
          }
          LOG_WRN("%s: xet download failed (%s); falling back to HTTPS\n",
                  __func__, result.error.c_str());
      }
  #endif
      // existing std::async fanout follows here, UNCHANGED
  ```

- [ ] **Step 12.4: Manual verification**

  With a Xet-backed repo and `LLAMA_XET=ON`:
  ```bash
  rm -rf ~/.cache/huggingface/hub/models--<org>--<repo>
  ./build/bin/llama-cli -hf <xet-backed repo> -n 1 -p "hi"
  ```
  Expected: download via Xet, model loads, inference runs.

  With a non-Xet repo (same build):
  ```bash
  ./build/bin/llama-cli -hf <old-lfs repo> -n 1 -p "hi"
  ```
  Expected: falls back to cpp-httplib (visible in logs at verbose), download succeeds.

- [ ] **Step 12.5: Commit**

  ```
  common : wire xet download into common_download_model (#<issue>)
  ```

---

## Task 13: Fallback tests

**Files:**
- Create: `tests/test-xet-fallback.cpp`
- Modify: `tests/CMakeLists.txt`

Goal: every fallback path in spec §7 has a test. This is where to exercise the "Xet fails → cpp-httplib succeeds → user gets their model" story.

- [ ] **Step 13.1: Test harness**

  The approach is to link a **stub Rust crate** (or a linker-visibility trick) that replaces the real `llama_xet_*` symbols with test-controllable versions. Simplest: compile `test-xet-fallback` with `LLAMA_USE_XET` defined but link it against a hand-rolled `test-llama-xet-stub.c` instead of the real Rust staticlib. Each test sets a flag that the stub checks:

  `tests/test-llama-xet-stub.c`:
  ```c
  #include "llama-xet.h"
  #include <stdlib.h>
  #include <string.h>

  int  test_session_new_mode     = 0; // 0=ok, 1=fail
  int  test_download_return_code = 0;

  LlamaXetSession * llama_xet_session_new(const char *a, const char *b, const char *c) {
      (void)a; (void)b; (void)c;
      if (test_session_new_mode) return NULL;
      return (LlamaXetSession *)0xDEADBEEF;
  }
  void llama_xet_session_free(LlamaXetSession *s) { (void)s; }
  void llama_xet_session_abort(LlamaXetSession *s) { (void)s; }

  int32_t llama_xet_download_files(LlamaXetSession *s, const LlamaXetFileInfo *f,
                                   size_t n, LlamaXetProgressFn cb, void *ud) {
      (void)s; (void)f; (void)n; (void)cb; (void)ud;
      return test_download_return_code;
  }

  const char * llama_xet_last_error(void) { return "test error"; }
  ```

  Gate tests by `test_*_mode` variable, call the real `common_download_model` path, assert that falls-back-to-cpp-httplib behavior is observable in logs or in a completed download.

- [ ] **Step 13.2: Add tests for each §7 row**

  - `session_new fails` → fallback
  - `download_files returns LXET_ERR_NETWORK` → fallback
  - `download_files returns LXET_ERR_INTEGRITY` → fallback
  - `download_files returns LXET_ERR_AUTH` → fallback
  - `download_files returns LXET_ERR_CANCELLED` → do NOT fall back; propagate to caller
  - `all_files_xet_backed returns false` → skip Xet path entirely, no warn
  - `LLAMA_XET=OFF` (compile-time) → Xet code is absent from the binary (verify with `nm`)

  Each test is short. Keep assertions simple: "the downloaded file exists at expected path" + log-string checks.

- [ ] **Step 13.3: Commit**

  ```
  tests : add Xet fallback test suite (#<issue>)
  ```

---

## Task 14: Opt-in integration test against real HF

**Files:**
- Create: `tests/test-xet-e2e.cpp` (gated by `LLAMA_XET_E2E=1` env var)
- Modify: `tests/CMakeLists.txt`

Goal: one end-to-end test that actually hits HF and confirms a real Xet-backed file downloads. Not in default CI — opt-in only.

- [ ] **Step 14.1: Small public Xet-backed GGUF**

  Pick a small (<50 MB) public GGUF you confirmed is Xet-backed in step P4. Hardcode the org/repo/file in the test.

- [ ] **Step 14.2: Test body**

  ```cpp
  #include "download.h"
  #include <cstdlib>
  #include <cassert>

  int main() {
      if (!std::getenv("LLAMA_XET_E2E")) return 0; // skip silently

      common_params_model model{};
      model.hf_repo = "<org>/<repo>";    // from step P4

      common_download_opts opts{};
      opts.bearer_token = std::getenv("HF_TOKEN") ? std::getenv("HF_TOKEN") : "";

      auto result = common_download_model(model, opts, false);
      assert(!result.model_path.empty());

      // Check cache layout: snapshot symlink → blob.
      // (path assertions specific to your OS)
      return 0;
  }
  ```

- [ ] **Step 14.3: Commit**

  ```
  tests : add opt-in end-to-end Xet download test (#<issue>)
  ```

---

## Task 15: Feature documentation — `docs/xet.md` + `docs/build.md` update

**Files:**
- Create: `docs/xet.md` (mirrors shape of `docs/llguidance.md`)
- Modify: `docs/build.md` (mention `-DLLAMA_XET=ON` near other optional flags)

Goal: users and maintainers can read one page and understand what the flag does.

- [ ] **Step 15.1: `docs/xet.md`** — short. One-paragraph overview (what Xet is), one code block showing the CMake flag, one paragraph on fallback behavior, one paragraph on cache location, link to xet-core.

- [ ] **Step 15.2: `docs/build.md`** — add a one-line entry in the optional features list: "`LLAMA_XET=ON` — use hf-xet for Hugging Face model downloads; see `docs/xet.md`."

- [ ] **Step 15.3: Commit**

  ```
  docs : add LLAMA_XET build flag documentation (#<issue>)
  ```

---

## Task 16: CI matrix row

**Files:**
- Modify: one file under `.github/workflows/` — probably `build.yml` (find the Linux x86_64 matrix and add `LLAMA_XET=ON` as a build-only variant)

Goal: CI compiles the `LLAMA_XET=ON` build on every PR, catching Rust or CMake regressions. Does NOT run end-to-end tests — those are opt-in (Task 14).

- [ ] **Step 16.1: Read the current build.yml matrix**

  Identify the Linux x86_64 Ubuntu job. Add a sibling entry or an axis value that sets `LLAMA_XET=ON`. Ensure the Rust toolchain is installed via `rustup` step (add one if absent — llguidance already triggers this).

- [ ] **Step 16.2: Verify**

  Push the branch, watch the CI run. `LLAMA_XET=ON` row must build to completion.

- [ ] **Step 16.3: Commit**

  ```
  ci : add LLAMA_XET=ON build variant for Linux (#<issue>)
  ```

---

## Task 17: Benchmark harness

**Files:**
- Create: `scripts/bench-xet.sh` (or `scripts/bench-xet.py` if you prefer)

Goal: one script that runs all six benchmark scenarios from spec §9 and emits a Markdown table for the PR description.

- [ ] **Step 17.1: Scenarios**

  Implement each row of the spec §9 table:

  1. Cold-cache large multipart Xet GGUF
  2. Warm-cache same model
  3. Warm-cache different quant of same base
  4. Connection-kill mid-stream (use `tc` to inject drops or kill network after N seconds)
  5. Non-Xet repo (confirms fallback is zero-cost)
  6. `LLAMA_XET=OFF` build size (just `du -h` of the binary)

- [ ] **Step 17.2: Output**

  Print a Markdown table matching the PR's "Results" section. Include mean and stddev over 5 runs. Save raw run data to `bench-xet-results/YYYYMMDD-HHMMSS/`.

- [ ] **Step 17.3: Commit**

  ```
  scripts : add xet download benchmark harness (#<issue>)
  ```

---

## Task 18: Open the upstream issue

**No files modified.** This task is the social one.

- [ ] **Step 18.1: Draft the issue** — title, context, linked llguidance precedent, benchmark plan summary, scope boundary (download-only), opt-in via `LLAMA_XET=OFF`. Write it in your voice, not copy-pasted from this plan.

- [ ] **Step 18.2: File at `ggml-org/llama.cpp`** — tag as `enhancement`, reference no existing issue.

- [ ] **Step 18.3: Wait for maintainer response.** If they decline or ask for major redesign, halt — do not open a draft PR. If they encourage it, proceed to Task 19.

---

## Task 19: Draft PR (build-wiring only, no behavior change)

Branch off **upstream/master** (NOT `design/xet-integration` — that's the private notes branch). Cherry-pick the commits from Tasks 1–7 (CMake, Rust crate, C++ header). Open a draft PR with the title `common : add optional hf-xet download backend (#<issue>)`.

- [ ] **Step 19.1: Create the PR branch**

  ```bash
  git fetch upstream
  git checkout -b feature/hf-xet-download-backend upstream/master
  git cherry-pick <task-1-sha>..<task-7-sha>
  git push -u origin feature/hf-xet-download-backend
  ```

- [ ] **Step 19.2: Open the draft PR**

  Body includes:
  - One-paragraph summary
  - Link to the issue from Task 18
  - Note that this is a draft with build-wiring only; behavior change follows in a subsequent commit
  - **AI-assistance disclosure**: "Investigation and design exploration used AI assistance. All code and prose authored by me; I can defend every line without AI help. The design document (private) references are at `docs/superpowers/specs/...` on a local `design/xet-integration` branch, not included in this PR."
  - Benchmark plan table from spec §9

- [ ] **Step 19.3: Address review comments in your own voice.** `AGENTS.md` is explicit that AI-generated reviewer responses will cause the PR to be closed.

---

## Task 20: Full-behavior commit on the same PR

Once Task 19's wiring review is green or converging, push Tasks 8–16 as additional commits on the same branch.

- [ ] **Step 20.1: Cherry-pick or replay commits 8–16 onto `feature/hf-xet-download-backend`.**
- [ ] **Step 20.2: Update PR description with benchmark results (Task 17 output).**
- [ ] **Step 20.3: Mark PR ready for review.**

---

## Self-review (plan coverage against spec)

Running the checklist myself before handoff:

| Spec section | Task(s) | Coverage |
|---|---|---|
| §1 Context / §2 Goals / §3 Non-goals | — | No code tasks; framing only. |
| §4 Architecture | 1–12 | All layers (C++/Rust/xet-core) exercised. |
| §5.1 CMake | 1, 3 | Option, ExternalProject, header. |
| §5.2 Rust crate | 1, 2, 3, 4, 5, 6 | Scaffolding → xet-pin → cbindgen → error → session → download. |
| §5.3 C ABI | 4, 5, 6 | All FFI fns implemented and header-verified. |
| §5.4 C++ integration | 7, 10, 11, 12 | Header, bridge, progress, orchestrator hook. |
| §5.5 HF metadata | 8, 9 | xet_hash parse + token fetch. |
| §6 Data flow | 10, 12 (manual verify) | End-to-end runs in Task 12.4 verify the sequence. |
| §7 Error handling | 13 | Every row in §7 → a fallback test. |
| §8 Tests | 4, 5, 6, 8, 13, 14 | Rust unit, C++ mock, opt-in E2E. |
| §9 Benchmark | 17 | Harness produces the spec's table. |
| §10 PR strategy | 18, 19, 20 | Issue-first, draft-first, AI disclosure. |
| §11 Risks | Preflight P1–P3 | xet_hash schema verified before coding. |
| §12 Follow-ups | — | Explicitly out-of-scope. |

No gaps. No `TBD`s. Code examples in every step that touches code.

Type/name consistency pass: `LlamaXetSession`, `LlamaXetFileInfo`, `LlamaXetProgressFn`, `llama_xet_*` functions — all match across tasks.

---

## Handoff

**This plan is a reading document for a human contributor.** The default skill offers "subagent-driven execution" or "inline execution" for implementation — **neither applies here** because llama.cpp's `AGENTS.md` forbids AI-generated code in upstream PRs. The contributor implements each task manually, using this document as scaffolding.

Recommended cadence:
- Treat each Task as a session (1–2 hours). Start with a fresh read of the Task's goal + files + steps.
- Commit at each "Commit" step. Don't batch commits — small commits are what llama.cpp reviewers prefer anyway.
- After Task 7 (the C++ header landing), pause and open the upstream issue (Task 18) before pushing further.
- After Task 12 (orchestrator hook), manually run `llama-cli -hf <xet-repo>` and confirm everything works end-to-end before Task 13.

Ship it with care.
