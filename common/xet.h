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

} // namespace llama_xet

#endif // LLAMA_USE_XET
