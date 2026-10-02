# Observed OAuth gateway header patch

`standard-headers.patch` adds unchanged forwarding of `Mcp-Method` and `Mcp-Name` to the observed gateway0.1.2. Repeated values are retained so the Rust MCP transport can reject duplicate singleton headers. The existing four-header loop, backend credential replacement, token expiry checks and Origin rejection remain unchanged. No request-body rewriting or validation bypass is added.

The pinned source hashes in `baseline.json` match both the local original gateway and the Pi host source. The source is a separate user-owned project and is not vendored here. The patch has **not been applied to the operational gateway**. The current restriction on operational switching still applies.

## Validate a copy

With cached Rust dependencies and an exclusively owned disposable FalkorDB reachable on loopback port16379:

```sh
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 \
  python3 scripts/validate-oauth-gateway-headers.py \
  --source /path/to/observed/oauth-gateway --baseline --report /private/before.json
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 \
  python3 scripts/validate-oauth-gateway-headers.py \
  --source /path/to/observed/oauth-gateway --report /private/after.json
```

The helper validates four input-file hashes, applies the patch only in a temporary copy, adds the actual Graphiti Rust project as a test dependency, and compiles the HTTP integration fixture inside that copy. It uses an in-memory OAuth database, synthetic tokens, loopback upstream and the read-only compatibility engine. It clears inherited provider credentials from the test environment and verifies the input source is unchanged afterward. The baseline reproduces three modern request failures; the patched run passes fifteen gateway tests, including eighteen request cases. It does not deploy, log in to GitHub or perform memory writes.

The actual ARM64 binaries were also compared on Pi4 against the observed gateway image with an empty dedicated database, internal network and separate synthetic OAuth state. Thirty-six cases pass, including matching ten-tool schemas, legacy requests and missing/mismatched/duplicate headers. See [evidence](../../docs/compat/oauth-header-results.md). These externally sourced gateway tests are a separate validation step; the repository's ordinary CI does not contain the original gateway source or run this helper.
