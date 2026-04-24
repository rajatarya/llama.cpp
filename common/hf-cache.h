#pragma once

#include <nlohmann/json_fwd.hpp>

#include <string>
#include <vector>

// Ref: https://huggingface.co/docs/hub/local-cache.md

namespace hf_cache {

struct hf_file {
    std::string path;
    std::string url;
    std::string local_path;
    std::string final_path;
    std::string oid;
    std::string repo_id;
    std::string revision;   // commit SHA this file was listed under
    size_t size = 0;        // only for the migration

    // Xet content-addressed hash, populated from the HF tree API
    // `xetHash` field when a file is Xet-backed. Empty otherwise.
    // Used by the LLAMA_XET download path; not used by the cpp-httplib
    // path.
    std::string xet_hash;
};

using hf_files = std::vector<hf_file>;

// Scoped Xet-CAS access token fetched from HF.
struct hf_xet_token {
    std::string access_token;     // empty on failure / non-Xet repo
    uint64_t    expiry_unix_secs = 0;
    std::string cas_url;
};

// Get files from HF API
hf_files get_repo_files(
    const std::string & repo_id,
    const std::string & token
);

// Fetch a scoped Xet-CAS token for a given repo + revision.
// On any failure (non-Xet repo, 404, network, schema mismatch),
// returns a token with empty access_token; callers should check
// .access_token.empty() and fall back to the plain HTTPS path.
hf_xet_token get_xet_token(
    const std::string & repo_id,
    const std::string & rev,
    const std::string & token
);

// Testable helpers: parse an HF xet-read-token response into hf_xet_token.
// The json-body overload is canonical; the string overload is a thin
// convenience that parses + delegates (used by the unit tests so they
// can exercise malformed-JSON handling).
hf_xet_token parse_xet_token_response(const nlohmann::json & j);
hf_xet_token parse_xet_token_response(const std::string &  body);

// True iff the file list is non-empty AND every entry has a populated
// xet_hash. Used by the orchestrator to decide whether the whole
// batch can go through the llama-xet fast path.
inline bool all_files_xet_backed(const hf_files & files) {
    if (files.empty()) return false;
    for (const auto & f : files) {
        if (f.xet_hash.empty()) return false;
    }
    return true;
}

hf_files get_cached_files(const std::string & repo_id = {});

// Create snapshot path (link or move/copy) and return it
std::string finalize_file(const hf_file & file);

// TODO: Remove later
void migrate_old_cache_to_hf_cache(const std::string & token, bool offline = false);

} // namespace hf_cache
