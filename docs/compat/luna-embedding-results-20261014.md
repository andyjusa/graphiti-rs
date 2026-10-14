# Luna gateway embedding cutover — 2026-10-14

The operating Raspberry Pi Graphiti backend now uses FactChat's OpenAI-compatible `/v1/gateway/embeddings/` endpoint with the existing Luna API key. The model remains `gemini-embedding-2`, with 3072-dimensional vectors and the existing raw-text profile. No original vectors were regenerated.

## Request and compatibility

Set `COMPAT_GEMINI_TRANSPORT=factchat` and `COMPAT_GEMINI_BASE=https://factchat-cloud.mindlogic.ai/v1/gateway`. The gateway uses `COMPAT_LLM_KEY_FILE` for Bearer authentication, and does not require or mount the Google key. Native transport remains the default for existing configurations and rollback.

The request fixes `model=gemini-embedding-2`, `dimensions=3072`, `encoding_format=float`; it replaces LF with a space, leaves CR unchanged, omits `task_type`, and retains batches of eight and the 256-input limit. Responses must name the same model and return every batch index exactly once. Input order is restored from indices; wrong dimensions, nonfinite/zero vectors and malformed responses fail closed. HTTP errors and redirects are not automatically retried.

Two bounded English/Korean synthetic inputs produced exactly the same vectors through Google native and the gateway: both cosine similarities were 1.0 and both maximum absolute differences were 0.0. This checks the observed transport/profile behavior for these inputs, not broad model-quality equivalence. [Official embedding API documentation](https://docs.mindlogic.ai/docs/general/api-gateway/reference/embeddings).

## Verification

- Formatting, all-target/all-feature Clippy with warnings denied, and 37 nonignored all-feature tests passed. Thirty existing DB-dependent cases remained explicitly ignored in the local run.
- Five independent gateway tests exercise wire/authentication, LF cleanup, 8+1 batching, reverse response order, empty/oversized inputs, malformed response fields/vectors, HTTP failures, redirects and operational endpoint/key guards.
- The all-feature ARM64 release binary and bounded embedding-only probe built successfully. The probe used the actual gateway, returned two valid 3072-dimensional vectors and accessed no DB.
- After draining existing ingestion, only the Rust backend was recreated. Its configured transport and binary hash were checked. Existing English/Korean memory queries returned the same five fact identities per query, an existing episode retained the same content hash, and the ten-tool MCP schema stayed identical.
- Two synthetic English/Korean episodes in a fresh exclusively owned group reached durable `committed` state. Episode reads and fact/node searches all passed. The test group and its telemetry key were removed after clearing its records; only two `cleared` audit receipts remain in the durable journal.
- The original FalkorDB, OAuth gateway, stateless bridge and ingress container identities/start times were unchanged. Backend status was `ok`.

Operating binary SHA-256: `a934608faa848525479e405a3803b5b493befb1b63e2a25d94bbcaa4fd45ea93`.

## Rollback

The previous immutable runtime image, compose file and compose environment were retained privately on the Pi, together with a backend-only rollback script. Rollback restores native Google transport and the prior key mount while retaining the same DB and durable journal. The rollback was prepared but was not needed or executed during the successful cutover.
