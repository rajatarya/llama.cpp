# LLAMA_XET benchmark results

Host: Darwin 25.4.0 Darwin Kernel Version 25.4.0: Thu Mar 19 19:33:50 PDT 2026; root:xnu-12377.101.15~1/RELEASE_ARM64_T6050 arm64
Branch: prototype/xet-integration
Commit: 339a3aac4
Date:   2026-04-24T01:14:34Z
Runs per cell: 3

## Scenario 1 — Cold cache download

Repo: `ggml-org/gemma-3-1b-it-GGUF` 

| Build | Wall clock |
|---|---|
| LLAMA_XET=OFF (cpp-httplib) | 21.92s ± 14.71s |
| LLAMA_XET=ON  (hf-xet)      | 17.35s ± 11.90s |

## Scenario 2 — Warm cache re-download (cache-hit short-circuit)

| Build | Wall clock |
|---|---|
| LLAMA_XET=OFF | 1.56s ± 1.04s |
| LLAMA_XET=ON  | 1.68s ± 1.14s |

## Scenario 5 — Non-Xet repo (fallback cost)

Downloading a legacy LFS-only GGUF with LLAMA_XET=ON should take
the cpp-httplib fallback path and match the LLAMA_XET=OFF timing
(no Xet overhead when Xet isn't usable).

TODO: pick a non-Xet repo (check `curl … /tree/main` — file with
no `xetHash` field). Skipped unless BENCH_NONXET_REPO is set.

## Scenario 6 — Binary size delta

| Build | llama-cli size |
|---|---|
| LLAMA_XET=OFF | 1.36 MB |
| LLAMA_XET=ON  | 1.36 MB |

