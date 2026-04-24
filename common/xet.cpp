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

// Thunk that trampolines the C callback from llama-xet into a
// C++ common_download_callback*. user_data is the callback pointer.
extern "C" void xet_progress_thunk(void * user_data,
                                   uint64_t completed_bytes,
                                   uint64_t total_bytes) {
    auto * cb = static_cast<common_download_callback *>(user_data);
    if (!cb) return;
    common_download_progress p;
    p.url        = "xet://batch";
    p.downloaded = static_cast<size_t>(completed_bytes);
    p.total      = static_cast<size_t>(total_bytes);
    p.cached     = false;
    cb->on_update(p);
}

try_result try_xet_download(
    const hf_cache::hf_files &      files,
    const hf_cache::hf_xet_token &  xet_token,
    const std::string &             bearer_token,
    const std::string &             token_refresh_url,
    common_download_callback *      progress_cb)
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

    // Start-of-batch progress event so the CLI progress bar initializes.
    if (progress_cb) {
        common_download_progress p0;
        p0.url    = "xet://batch";
        p0.total  = 0;
        for (const auto & f : files) p0.total += f.size;
        progress_cb->on_start(p0);
    }

    int32_t rc = llama_xet_download_files(
        session.get(), infos.data(), infos.size(),
        progress_cb ? xet_progress_thunk : nullptr,
        static_cast<void *>(progress_cb));
    if (rc != LXET_OK) {
        r.error = "llama_xet_download_files rc=" + std::to_string(rc) + ": " + last_error();
        if (progress_cb) {
            common_download_progress p_fail;
            p_fail.url        = "xet://batch";
            p_fail.downloaded = 0;
            p_fail.total      = 0;
            for (const auto & f : files) p_fail.total += f.size;
            progress_cb->on_done(p_fail, /*ok=*/false);
        }
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
    //
    // If one rename fails partway through, roll back the earlier
    // successful renames so the caller sees a clean "no files moved"
    // state. Without this, the cpp-httplib fallback would see some
    // final-path files present (which it would skip as "already done")
    // and others missing — a messy mixed state. Rollback trades a bit
    // of extra work for deterministic failure semantics.
    size_t renamed = 0;
    for (size_t i = 0; i < in_progress_paths.size(); ++i) {
        std::error_code ec;
        fs::rename(in_progress_paths[i], files[i].local_path, ec);
        if (ec) {
            r.error = "rename(" + in_progress_paths[i] + " -> " + files[i].local_path
                    + "): " + ec.message();
            // Roll back any successful renames.
            for (size_t j = 0; j < renamed; ++j) {
                std::error_code rbc;
                fs::rename(files[j].local_path, in_progress_paths[j], rbc);
                // Ignore rollback errors — we're already on the failure
                // path and best-effort cleanup is the most we can do.
            }
            return r;
        }
        ++renamed;
    }

    if (progress_cb) {
        common_download_progress p_done;
        p_done.url = "xet://batch";
        p_done.downloaded = 0;
        p_done.total      = 0;
        for (const auto & f : files) {
            p_done.downloaded += f.size;
            p_done.total      += f.size;
        }
        progress_cb->on_done(p_done, /*ok=*/true);
    }

    r.ok = true;
    return r;
}

} // namespace llama_xet

#endif // LLAMA_USE_XET
