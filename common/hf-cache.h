#pragma once

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
    size_t size = 0; // only for the migration

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

// Testable helper: parse the body of an HF xet-read-token response.
// Returns a token with empty fields on any schema mismatch.
hf_xet_token parse_xet_token_response(const std::string & body);

hf_files get_cached_files(const std::string & repo_id = {});

// Create snapshot path (link or move/copy) and return it
std::string finalize_file(const hf_file & file);

// TODO: Remove later
void migrate_old_cache_to_hf_cache(const std::string & token, bool offline = false);

} // namespace hf_cache
