# Verification record — 2026-10-01 KST

Historical record. Current isolated Pi/CI/search results are in [compatibility verification](compat/verification.md), with [fulltext and synthetic restore evidence](compat/fulltext-results.md). The measurements below retain their original scope and source date.

Scope: source implementation, disposable local FalkorDB, and Linux ARM64 containers. No existing Raspberry Pi service or production memory was changed.

## Automated checks

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed; database/local-model cases are opt-in.
- `LOCAL_TEST_MODEL_DIR=models/minilm cargo test --all-features -- --include-ignored`: **27 passed, 0 failed, 0 ignored**.
- Real FalkorDB 4.14.11: concurrent idempotency, group isolation, shared-provenance deletion, explicit temporal replacement, embedding-space conflict, and failed-ingestion metadata invariants.
- MCP: standard initialization/discovery/ingestion/search over an in-memory duplex transport; HTTP authentication and initialization; retries avoid a second extraction call; concurrent searches reject excess work before queuing.
- Model API: actual local HTTP responses exercise extraction parsing and duplicate embedding-index rejection, without paid API calls.
- Local embeddings: real pinned all-MiniLM-L6-v2 produces normalized semantic vectors; tokenizer changes alter model identity; missing model files are diagnosed.

## ARM64 packaging

Both Docker builds succeeded with `--platform linux/arm64`, with and without `CARGO_FEATURES=local-embeddings`. Images report `arm64 linux`. The authenticated HTTP example ingests and searches facts in the default ARM64 image.

The default release binary is **8,529,464 bytes**. A single Docker stats sample after the small example reported **1.68 MiB** for the default server container. This excludes FalkorDB, image disk usage, and any embedding model, and is not a sustained-workload or Raspberry Pi benchmark.

Local smoke-test model revision: `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`. The optional local ARM64 container also ingested and retrieved a fact with a 384-dimensional embedding. Its release binary is **12,678,544 bytes**; a post-example Docker stats sample reported **101.1 MiB** with MiniLM loaded, excluding FalkorDB. This is an ARM64 Docker Desktop VM result, not a physical Pi benchmark.

## Review and limitations

One independent whole-tree review identified search admission, external endpoint identity, local artifact identity, and failed-ingestion metadata side effects. Regression tests reproduced each behavior before its fix; the complete suite passed afterward. Compose resource settings and architecture documentation were reconciled with the final implementation.

Physical Raspberry Pi RAM/latency, ARMv7, remote paid APIs, Python Graphiti migration, and compatibility with the existing Pi's custom server/discover handshake remain unverified. GitHub CI is configured separately; local results above do not imply that a remote CI run passed.
