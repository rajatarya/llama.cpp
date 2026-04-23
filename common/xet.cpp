#include "xet.h"

#ifdef LLAMA_USE_XET

#include "hf-cache.h"
#include "log.h"

#include <cstdint>
#include <filesystem>
#include <string>
#include <system_error>
#include <vector>

namespace llama_xet {

namespace fs = std::filesystem;

static std::string append_suffix(const std::string & path, const char * sfx) {
    return path + sfx;
}

try_result try_xet_download(
    const hf_cache::hf_files &      files,
    const hf_cache::hf_xet_token &  xet_token,
    const std::string &             bearer_token,
    const std::string &             token_refresh_url)
{
    try_result r;

    if (files.empty()) {
        r.ok = true;
        return r;
    }
    if (xet_token.access_token.empty() || xet_token.cas_url.empty()) {
        r.error = "no xet read-token available";
        return r;
    }

    // Create parent dirs (xet-core requires them to exist) and build
    // the .xetInProgress dest paths. Holding the strings in owned
    // vectors keeps their c_str() pointers valid through the Rust call.
    std::vector<std::string>       hashes;
    std::vector<std::string>       in_progress_paths;
    std::vector<LlamaXetFileInfo>  infos;
    hashes.reserve(files.size());
    in_progress_paths.reserve(files.size());
    infos.reserve(files.size());

    for (const auto & f : files) {
        if (f.xet_hash.empty()) {
            r.error = "file '" + f.path + "' has no xet_hash — caller should not have taken xet path";
            return r;
        }
        fs::path local(f.local_path);
        std::error_code ec;
        fs::create_directories(local.parent_path(), ec);
        if (ec) {
            r.error = "create_directories(" + local.parent_path().string() + "): " + ec.message();
            return r;
        }

        hashes.push_back(f.xet_hash);
        in_progress_paths.push_back(append_suffix(f.local_path, ".xetInProgress"));

        LlamaXetFileInfo info{};
        info.hash      = hashes.back().c_str();
        info.file_size = static_cast<uint64_t>(f.size);
        info.dest_path = in_progress_paths.back().c_str();
        infos.push_back(info);
    }

    // Session is scoped to this call (per the spec's per-download
    // lifecycle decision). Freed on scope exit via RAII.
    auto session = new_session(
        xet_token.cas_url.c_str(),
        !xet_token.access_token.empty() ? xet_token.access_token.c_str() : bearer_token.c_str(),
        token_refresh_url.empty() ? nullptr : token_refresh_url.c_str());
    if (!session) {
        r.error = "session_new failed: " + last_error();
        return r;
    }

    // Progress callback wired in Task 11 — no-op for now so Task 10
    // is isolated from the common_download_opts progress integration.
    int32_t rc = llama_xet_download_files(
        session.get(), infos.data(), infos.size(),
        /*progress_cb*/ nullptr, /*user_data*/ nullptr);
    if (rc != LXET_OK) {
        r.error = "llama_xet_download_files rc=" + std::to_string(rc) + ": " + last_error();
        // Best-effort cleanup of any partial .xetInProgress files so the
        // cpp-httplib fallback doesn't mistake them for resume state.
        for (const auto & p : in_progress_paths) {
            std::error_code ec;
            fs::remove(p, ec);   // ignore errors — file may not exist
        }
        return r;
    }

    // Atomic rename each .xetInProgress to its final local_path.
    // hf_cache::finalize_file (called by the orchestrator after this)
    // will then create the snapshots/ symlink.
    for (size_t i = 0; i < in_progress_paths.size(); ++i) {
        std::error_code ec;
        fs::rename(in_progress_paths[i], files[i].local_path, ec);
        if (ec) {
            r.error = "rename(" + in_progress_paths[i] + " -> " + files[i].local_path
                    + "): " + ec.message();
            return r;
        }
    }

    r.ok = true;
    return r;
}

} // namespace llama_xet

#endif // LLAMA_USE_XET
