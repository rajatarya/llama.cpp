// Unit tests for the Xet orchestrator decision logic in common/hf-cache.
//
// Covers the pure helpers that decide whether to take the Xet fast path
// vs fall back to cpp-httplib:
//   - hf_cache::all_files_xet_backed
//   - hf_cache::parse_xet_token_response
//
// Integration tests (real HF downloads, session lifecycle, progress
// callback delivery) live in test-xet-e2e.cpp behind LLAMA_XET_E2E=1
// and the Rust-side unit tests live in common/llama-xet/src/.

#include "hf-cache.h"

#include <string>

#undef NDEBUG
#include <cassert>
#include <cstdio>

static void test_all_files_xet_backed_empty_list_is_false() {
    hf_cache::hf_files files;
    assert(!hf_cache::all_files_xet_backed(files));
}

static void test_all_files_xet_backed_single_populated() {
    hf_cache::hf_files files;
    hf_cache::hf_file f;
    f.xet_hash = "deadbeef";
    files.push_back(f);
    assert(hf_cache::all_files_xet_backed(files));
}

static void test_all_files_xet_backed_one_missing_disqualifies_batch() {
    hf_cache::hf_files files;
    hf_cache::hf_file a; a.xet_hash = "aa";
    hf_cache::hf_file b; b.xet_hash = "";      // legacy LFS file
    hf_cache::hf_file c; c.xet_hash = "cc";
    files.push_back(a);
    files.push_back(b);
    files.push_back(c);
    assert(!hf_cache::all_files_xet_backed(files));
}

static void test_parse_xet_token_happy_path() {
    auto tok = hf_cache::parse_xet_token_response(
        R"({"accessToken":"eyJabc","exp":1734567890,"casUrl":"https://cas.example.com/"})");
    assert(tok.access_token     == "eyJabc");
    assert(tok.expiry_unix_secs == 1734567890ULL);
    assert(tok.cas_url          == "https://cas.example.com/");
}

static void test_parse_xet_token_missing_fields_are_empty() {
    auto tok = hf_cache::parse_xet_token_response(R"({"accessToken":"t"})");
    assert(tok.access_token     == "t");
    assert(tok.expiry_unix_secs == 0);
    assert(tok.cas_url.empty());
}

static void test_parse_xet_token_wrong_types_are_ignored() {
    auto tok = hf_cache::parse_xet_token_response(
        R"({"accessToken":42,"exp":"not-a-number","casUrl":null})");
    // None of the fields match their expected types; all stay empty.
    assert(tok.access_token.empty());
    assert(tok.expiry_unix_secs == 0);
    assert(tok.cas_url.empty());
}

static void test_parse_xet_token_malformed_json_is_safe() {
    auto tok = hf_cache::parse_xet_token_response("}}{not json{{");
    assert(tok.access_token.empty());
    assert(tok.expiry_unix_secs == 0);
    assert(tok.cas_url.empty());
}

static void test_parse_xet_token_empty_body_is_safe() {
    auto tok = hf_cache::parse_xet_token_response("");
    assert(tok.access_token.empty());
    assert(tok.expiry_unix_secs == 0);
    assert(tok.cas_url.empty());
}

static void test_parse_xet_token_non_object_root_is_safe() {
    auto tok = hf_cache::parse_xet_token_response(R"(["this","is","an","array"])");
    assert(tok.access_token.empty());
    assert(tok.expiry_unix_secs == 0);
    assert(tok.cas_url.empty());
}

int main() {
    printf("test-xet-fallback: decision logic for cpp-httplib fallback\n");

    test_all_files_xet_backed_empty_list_is_false();
    test_all_files_xet_backed_single_populated();
    test_all_files_xet_backed_one_missing_disqualifies_batch();
    test_parse_xet_token_happy_path();
    test_parse_xet_token_missing_fields_are_empty();
    test_parse_xet_token_wrong_types_are_ignored();
    test_parse_xet_token_malformed_json_is_safe();
    test_parse_xet_token_empty_body_is_safe();
    test_parse_xet_token_non_object_root_is_safe();

    printf("test-xet-fallback: all tests passed\n");
    return 0;
}
