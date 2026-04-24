// Opt-in end-to-end Xet download test.
//
// Exercises the full HF-to-disk path:
//   common_download_model  (orchestrator)
//     → get_hf_plan        (HF tree + xet-read-token)
//     → try_xet_download   (FFI to llama-xet Rust crate)
//     → XetSession / XetFileDownloadGroup in xet-core
//     → atomic rename + finalize_file (snapshot symlink)
//
// Gated on LLAMA_XET_E2E=1 so the default `ctest` run stays offline.
// Also skipped silently when LLAMA_USE_XET is not compiled in (so
// LLAMA_XET=OFF builds do not fail the suite).
//
// Uses ggml-org/gemma-3-1b-it-GGUF Q4_K_M (~806 MB). The download is
// cached under ~/.cache/huggingface/hub, so subsequent runs exercise
// the cache-hit short-circuit added in Task 12.
//
// To run:
//   LLAMA_XET_E2E=1 HF_TOKEN=<your-token> ./build/bin/test-xet-e2e

#include "common.h"
#include "download.h"

#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>

#undef NDEBUG
#include <cassert>

static bool env_truthy(const char * name) {
    const char * v = std::getenv(name);
    return v && *v && std::string(v) != "0";
}

int main() {
    if (!env_truthy("LLAMA_XET_E2E")) {
        printf("test-xet-e2e: LLAMA_XET_E2E not set; skipping\n");
        return 0;
    }

#ifndef LLAMA_USE_XET
    printf("test-xet-e2e: LLAMA_USE_XET not compiled in; skipping\n");
    return 0;
#else

    printf("test-xet-e2e: running against ggml-org/gemma-3-1b-it-GGUF\n");

    common_params_model model{};
    model.hf_repo = "ggml-org/gemma-3-1b-it-GGUF";
    model.hf_file = "gemma-3-1b-it-Q4_K_M.gguf";

    common_download_opts opts{};
    if (const char * tok = std::getenv("HF_TOKEN")) {
        opts.bearer_token = tok;
    }

    auto result = common_download_model(model, opts, /*download_mmproj=*/false);

    assert(!result.model_path.empty() && "download must produce a model_path");
    printf("test-xet-e2e: model_path = %s\n", result.model_path.c_str());

    // Verify the file is on disk and non-empty. Size is not asserted to
    // an exact number since this could run against future revisions of
    // the repo with different quant sizes.
    assert(std::filesystem::exists(result.model_path));
    auto sz = std::filesystem::file_size(result.model_path);
    printf("test-xet-e2e: file_size = %llu bytes\n", (unsigned long long) sz);
    assert(sz > 100 * 1024 * 1024 && "downloaded file should be > 100 MiB");

    printf("test-xet-e2e: all checks passed\n");
    return 0;
#endif
}
