#!/usr/bin/env bash
#
# Benchmark the LLAMA_XET download path against the cpp-httplib baseline.
#
# Requires two builds:
#   build-nox/  → cmake -B build-nox  -DLLAMA_XET=OFF && cmake --build build-nox  --target llama-cli -j
#   build-xet/  → cmake -B build-xet  -DLLAMA_XET=ON  && cmake --build build-xet  --target llama-cli -j
# This script will create them automatically if missing.
#
# Env:
#   HF_TOKEN                — required for private/gated repos
#   BENCH_REPO              — default ggml-org/gemma-3-1b-it-GGUF
#   BENCH_FILE              — optional specific file in the repo
#   BENCH_QUANT_B           — optional second repo/file for the "variant dedup" scenario
#   BENCH_RUNS              — number of timed runs per cell (default 3)
#   BENCH_OUT               — output Markdown file (default bench-xet-results.md)
#
# Scenarios run:
#   1. Cold cache download (the big one)
#   2. Warm cache re-download (cache hit should be instant for both)
#   3. Variant dedup (if BENCH_QUANT_B is set) — only xet benefits here
#   5. Non-Xet repo (fallback path; both builds should perform the same)
#   6. Binary size delta
#
# NOT included (out of scope for the prototype harness; add later):
#   - Scenario 4: mid-stream connection drop recovery (needs network injection)
#   - Bytes-over-the-wire dedup accounting (needs tcpdump or xet-core telemetry)

set -euo pipefail

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT"

BENCH_REPO="${BENCH_REPO:-ggml-org/gemma-3-1b-it-GGUF}"
BENCH_FILE="${BENCH_FILE:-}"
BENCH_QUANT_B="${BENCH_QUANT_B:-}"
BENCH_RUNS="${BENCH_RUNS:-3}"
BENCH_OUT="${BENCH_OUT:-bench-xet-results.md}"

HF_CACHE="${HF_HUB_CACHE:-$HOME/.cache/huggingface/hub}"

log() { printf '[bench-xet] %s\n' "$*" >&2; }

ensure_build() {
    local dir="$1" flag="$2"
    if [ ! -x "$dir/bin/llama-cli" ]; then
        log "building $dir with LLAMA_XET=$flag (one-time, few minutes)"
        cmake -B "$dir" -DLLAMA_XET="$flag" >/dev/null
        cmake --build "$dir" --target llama-cli -j >/dev/null
    fi
}

clear_cache() {
    # Blow away ONLY the repo under test to preserve other cached models.
    local repo_safe="${1//\//--}"
    rm -rf "$HF_CACHE/models--$repo_safe"
}

# Time one download-only run (download + model load + 1 token + exit).
# -st (--single-turn) makes llama-cli exit after one prompt without
# entering the interactive chat loop. Both the OFF and ON builds pay
# the same model-load + one-token cost, so the delta is attributable
# to the download phase.
time_one_download() {
    local build_dir="$1" repo="$2" file_arg="$3"
    local start end
    start=$(date +%s.%N)
    (
        </dev/null "$build_dir/bin/llama-cli" \
            -hf "$repo" ${file_arg:+-hff "$file_arg"} -hft "${HF_TOKEN:-}" \
            -n 1 -p "hi" --no-warmup -st >/dev/null 2>&1 || true
    )
    end=$(date +%s.%N)
    awk -v s="$start" -v e="$end" 'BEGIN { printf "%.2f", e - s }'
}

measure_cold() {
    local build_dir="$1" repo="$2" file_arg="$3"
    local samples=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        clear_cache "$repo"
        local t
        t=$(time_one_download "$build_dir" "$repo" "$file_arg")
        samples+=("$t")
        log "  cold run $i/$BENCH_RUNS: ${t}s"
    done
    printf '%s ' "${samples[@]}"
}

measure_warm() {
    local build_dir="$1" repo="$2" file_arg="$3"
    # Prime cache once outside the timed runs.
    clear_cache "$repo"
    time_one_download "$build_dir" "$repo" "$file_arg" >/dev/null
    local samples=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        local t
        t=$(time_one_download "$build_dir" "$repo" "$file_arg")
        samples+=("$t")
        log "  warm run $i/$BENCH_RUNS: ${t}s"
    done
    printf '%s ' "${samples[@]}"
}

mean_stddev() {
    # Reads samples on stdin, prints "mean ± stddev"
    awk 'BEGIN{n=0}
         { s+=$1; ss+=$1*$1; xs[n++]=$1 }
         END { if(n==0){print "n/a"; exit}
               m=s/n;
               v=0; for(i=0;i<n;i++) v+=(xs[i]-m)*(xs[i]-m); v=(n>1)?v/(n-1):0;
               printf "%.2fs ± %.2fs", m, sqrt(v) }'
}

binary_size() {
    local build_dir="$1"
    stat -f '%z' "$build_dir/bin/llama-cli" 2>/dev/null \
        || stat -c '%s' "$build_dir/bin/llama-cli"
}

human_bytes() {
    awk -v b="$1" 'BEGIN {
        split("B KB MB GB TB",u," ");
        i=1; while(b>=1024 && i<5){b/=1024;i++}
        printf "%.2f %s", b, u[i]
    }'
}

# ---- Go! ----

ensure_build build-nox OFF
ensure_build build-xet ON

{
    echo "# LLAMA_XET benchmark results"
    echo
    echo "Host: $(uname -srmv)"
    echo "Branch: $(git rev-parse --abbrev-ref HEAD)"
    echo "Commit: $(git rev-parse --short HEAD)"
    echo "Date:   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "Runs per cell: $BENCH_RUNS"
    echo
    echo "## Scenario 1 — Cold cache download"
    echo
    echo "Repo: \`$BENCH_REPO\` ${BENCH_FILE:+(file: \`$BENCH_FILE\`)}"
    echo
    echo "| Build | Wall clock |"
    echo "|---|---|"
} > "$BENCH_OUT"

log "== Scenario 1: cold cache =="
log "measuring LLAMA_XET=OFF (cpp-httplib baseline)"
nox_cold=$(measure_cold build-nox "$BENCH_REPO" "$BENCH_FILE")
echo "| LLAMA_XET=OFF (cpp-httplib) | $(echo "$nox_cold" | tr ' ' '\n' | mean_stddev) |" >> "$BENCH_OUT"

log "measuring LLAMA_XET=ON (Xet)"
xet_cold=$(measure_cold build-xet "$BENCH_REPO" "$BENCH_FILE")
echo "| LLAMA_XET=ON  (hf-xet)      | $(echo "$xet_cold" | tr ' ' '\n' | mean_stddev) |" >> "$BENCH_OUT"

{
    echo
    echo "## Scenario 2 — Warm cache re-download (cache-hit short-circuit)"
    echo
    echo "| Build | Wall clock |"
    echo "|---|---|"
} >> "$BENCH_OUT"

log "== Scenario 2: warm cache =="
log "measuring LLAMA_XET=OFF"
nox_warm=$(measure_warm build-nox "$BENCH_REPO" "$BENCH_FILE")
echo "| LLAMA_XET=OFF | $(echo "$nox_warm" | tr ' ' '\n' | mean_stddev) |" >> "$BENCH_OUT"
log "measuring LLAMA_XET=ON"
xet_warm=$(measure_warm build-xet "$BENCH_REPO" "$BENCH_FILE")
echo "| LLAMA_XET=ON  | $(echo "$xet_warm" | tr ' ' '\n' | mean_stddev) |" >> "$BENCH_OUT"

if [ -n "$BENCH_QUANT_B" ]; then
    {
        echo
        echo "## Scenario 3 — Variant dedup (different quant of same base)"
        echo
        echo "Primary: \`$BENCH_REPO\` ${BENCH_FILE:+(\`$BENCH_FILE\`)}"
        echo "Variant: \`$BENCH_QUANT_B\`"
        echo
        echo "Cache is primed with the PRIMARY, then the VARIANT is downloaded."
        echo "LLAMA_XET should fetch notably less bandwidth than LLAMA_XET=OFF."
        echo
        echo "| Build | Variant download time |"
        echo "|---|---|"
    } >> "$BENCH_OUT"

    log "== Scenario 3: variant dedup =="
    for build in nox xet; do
        flag=$([ "$build" = "nox" ] && echo OFF || echo ON)
        clear_cache "$BENCH_REPO"
        clear_cache "$BENCH_QUANT_B"
        # Prime the cache with the primary.
        time_one_download "build-$build" "$BENCH_REPO" "$BENCH_FILE" >/dev/null
        # Measure the variant's download time.
        samples=()
        for i in $(seq 1 "$BENCH_RUNS"); do
            clear_cache "$BENCH_QUANT_B"
            t=$(time_one_download "build-$build" "$BENCH_QUANT_B" "")
            samples+=("$t")
            log "  variant run $i/$BENCH_RUNS (build-$build): ${t}s"
        done
        echo "| LLAMA_XET=$flag | $(printf '%s\n' "${samples[@]}" | mean_stddev) |" >> "$BENCH_OUT"
    done
fi

{
    echo
    echo "## Scenario 5 — Non-Xet repo (fallback cost)"
    echo
    echo "Downloading a legacy LFS-only GGUF with LLAMA_XET=ON should take"
    echo "the cpp-httplib fallback path and match the LLAMA_XET=OFF timing"
    echo "(no Xet overhead when Xet isn't usable)."
    echo
    echo "TODO: pick a non-Xet repo (check \`curl … /tree/main\` — file with"
    echo "no \`xetHash\` field). Skipped unless BENCH_NONXET_REPO is set."
    echo
} >> "$BENCH_OUT"

if [ -n "${BENCH_NONXET_REPO:-}" ]; then
    log "== Scenario 5: non-Xet repo =="
    echo "| Build | Wall clock |" >> "$BENCH_OUT"
    echo "|---|---|" >> "$BENCH_OUT"
    for build in nox xet; do
        flag=$([ "$build" = "nox" ] && echo OFF || echo ON)
        s=$(measure_cold "build-$build" "$BENCH_NONXET_REPO" "")
        echo "| LLAMA_XET=$flag | $(echo "$s" | tr ' ' '\n' | mean_stddev) |" >> "$BENCH_OUT"
    done
fi

{
    echo "## Scenario 6 — Binary size delta"
    echo
    echo "| Build | llama-cli size |"
    echo "|---|---|"
    echo "| LLAMA_XET=OFF | $(human_bytes "$(binary_size build-nox)") |"
    echo "| LLAMA_XET=ON  | $(human_bytes "$(binary_size build-xet)") |"
    echo
} >> "$BENCH_OUT"

log "results written to $BENCH_OUT"
cat "$BENCH_OUT"
