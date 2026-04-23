#pragma once

// C++ RAII wrapper around the llama-xet C ABI (generated llama-xet.h).
//
// This header is a no-op unless LLAMA_USE_XET is defined — which CMake
// does whenever LLAMA_XET=ON. Consumers should guard includes with the
// same macro:
//
//     #ifdef LLAMA_USE_XET
//     #include "xet.h"
//     #endif
//
// Inside the guard, an RAII unique_ptr alias owns the opaque session
// handle, and a helper converts the thread-local last-error C string
// to std::string for ergonomic error reporting.

#ifdef LLAMA_USE_XET

#include "llama-xet.h"   // cbindgen-generated, on llama-common's include path
#include "hf-cache.h"    // hf_files, hf_xet_token

#include <memory>
#include <string>

namespace llama_xet {

struct session_deleter {
    void operator()(LlamaXetSession * s) const noexcept {
        if (s) llama_xet_session_free(s);
    }
};

using session_ptr = std::unique_ptr<LlamaXetSession, session_deleter>;

// Creates a new session. Returns an empty session_ptr on failure;
// call last_error() for the diagnostic.
inline session_ptr new_session(const char * endpoint,
                               const char * bearer_token,
                               const char * token_refresh_url) {
    return session_ptr(
        llama_xet_session_new(endpoint, bearer_token, token_refresh_url));
}

// Snapshots the current thread's last-error message as a std::string.
// Safe to call even when no error has been set (returns empty string).
inline std::string last_error() {
    const char * msg = llama_xet_last_error();
    return (msg && *msg) ? std::string(msg) : std::string();
}

// Result of a try_xet_download attempt. `ok=true` means every file
// was downloaded and moved to its hf_file::local_path. `ok=false`
// means the caller should fall back to the cpp-httplib path.
struct try_result {
    bool        ok = false;
    std::string error;
};

// Download a batch of Xet-backed files through the llama-xet FFI.
//
// Files must all have non-empty xet_hash (caller is expected to have
// verified this via all_files_xet_backed). Each file's bytes land at
// its `local_path`; the orchestrator is responsible for calling
// hf_cache::finalize_file afterwards to create snapshot symlinks.
//
// On failure: .xetInProgress staging files are removed, final
// local_path files are not touched; caller can safely fall through
// to the cpp-httplib path which will resume or start fresh.
try_result try_xet_download(
    const hf_cache::hf_files &     files,
    const hf_cache::hf_xet_token & xet_token,
    const std::string &            bearer_token,
    const std::string &            token_refresh_url);

} // namespace llama_xet

#endif // LLAMA_USE_XET
