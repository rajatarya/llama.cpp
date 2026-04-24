# Hugging Face Xet Support in llama.cpp

[Xet](https://huggingface.co/docs/hub/storage-backends#xet) is the content-addressed, chunk-level deduplicated storage backend behind a growing set of Hugging Face model repositories. When a repo is Xet-backed, downloads can skip any chunk that's already present in the user's local cache — across files, across quantizations of the same base model, and across unrelated repos that happen to share content.

The `LLAMA_XET` build option adds an optional fast path that routes HF model downloads through the [`hf-xet`](https://github.com/huggingface/xet-core) Rust crate. When Xet is unavailable (repo not Xet-backed, network error, schema mismatch, etc.), llama.cpp transparently falls back to the existing cpp-httplib download path with no behavior change.

## Building

To enable Xet support, build llama.cpp with the `LLAMA_XET` option:

```sh
cmake -B build -DLLAMA_XET=ON
cmake --build build -j
```

This requires the Rust compiler and the `cargo` tool to be [installed](https://www.rust-lang.org/tools/install). `LLAMA_XET=ON` also requires `LLAMA_OPENSSL=ON` (the default) — the HF metadata API is still fetched over HTTPS regardless of whether byte transfers go through Xet.

`LLAMA_XET=OFF` (the default) produces a binary bit-identical in behavior to the baseline build. No new runtime dependencies, no binary-size impact.

## Interface

No new command-line arguments. Existing `-hf`/`--hf-repo`, `-hff`/`--hf-file`, and `-hft`/`--hf-token` flags work as before. When the repo is Xet-backed, the download runs through xet-core automatically; otherwise the cpp-httplib path handles it.

## What you get

1. **Faster multi-file downloads.** Multipart GGUFs (`model-00001-of-00046.gguf`, …) benefit from chunk-level deduplication across all splits in a single download group.
2. **Faster re-downloads.** Chunks already present in `~/.cache/huggingface/hub/blobs/` are skipped.
3. **Faster variant downloads.** Downloading a different quantization of the same base model reuses chunks shared between the two quants.
4. **Better resilience.** Chunk-level retry and resume; a mid-stream connection failure doesn't restart the whole file.

## Fallback policy

Xet is a performance shortcut, never a hard dependency. If any of these fail, llama.cpp logs a warning at verbose level and falls back to the existing cpp-httplib HTTPS path:

- Repo is not Xet-backed (tree API returns no `xetHash` fields).
- `/api/models/{repo}/xet-read-token/{rev}` endpoint returns 404 / 5xx / network error.
- Session creation or download fails for any reason inside xet-core.

The fallback path is the ground truth. No user-facing error is ever "Xet failed" — the worst case is "Xet was skipped and we downloaded via HTTPS instead."

## Cache layout

Xet-backed downloads use the same on-disk layout as `huggingface_hub` and the existing cpp-httplib path:

```
~/.cache/huggingface/hub/models--{org}--{repo}/
├── blobs/
│   └── {sha256-oid}              ← actual file content
├── snapshots/
│   └── {commit}/
│       └── {file}.gguf → ../../blobs/{sha256-oid}  (relative symlink)
└── refs/
    └── main                      ← commit hash
```

Binaries built with different `LLAMA_XET` settings (or with `huggingface_hub` itself) read and share the same cache.

## Testing

Offline unit tests (run by default via `ctest`):

```sh
./build/bin/test-xet-fallback
```

Opt-in end-to-end test (hits real HF):

```sh
LLAMA_XET_E2E=1 HF_TOKEN=<your-token> ./build/bin/test-xet-e2e
```

Rust-side unit tests for the FFI crate:

```sh
cargo test --manifest-path common/llama-xet/Cargo.toml
```
