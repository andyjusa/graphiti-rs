# Raspberry Pi synthetic validation — 2026-10-02

**Latest:** [the global vector retrieval follow-up](vector-retrieval.md) fixes the outlier disagreement and passes 45 Pi/CI tests. The measurements below are historical, from source `9d642e06`; its miss is superseded by the follow-up evidence.

**Decision: standalone synthetic checks pass; Python-server replacement remains no-go.**

Source commit `9d642e06b25560ba32aaa01ea03766f680903db8`; release build with both optional features. Raspberry Pi4 ModelB Rev1.2, aarch64, 3795MiB RAM. A fresh, private DB directory and labelled internal network used the observed image/Redis8.2.3/FalkorDB module41606 and matching module hash. The operational DB, mounts, gateway, credentials and routes were not changed.

## Executed checks

- **42 distinct executable tests passed on the Pi**, including three compatibility DB/HTTP integrations, eleven standalone DB/HTTP integrations, two actual local MiniLM tests and 26 nonignored tests. Two of these execute 40 selected-source synthetic differential inputs. [Raw test log](pi-20261002/tests.txt).
- [CI on the same Rust source](https://github.com/andyjusa/graphiti-rs/actions/runs/36973858192) passes tests and ARM64 default/local/compatibility builds. CI uses module 4.14.11; Pi tests use the actual observed module 4.16.6.
- Fixed an actual module 4.16.6 reconnect failure: repeated range-index creation errored. Existing node RANGE indexes are verified by label/property/type before accepting that error.
- Fixed standalone shared-hub ranking: unrelated low-ID neighbors displaced an exact keyword seed outside the recent window. Expansion now retains direct relevance before its bound; the fixture MRR@10 improved from **0.575 to 1.0**. A dedicated regression uses a matching fact beyond the 200-fact recent window.

## Measurements

These are single-run synthetic measurements, not an approved production budget. DB was limited to one CPU/512MiB; Rust used its default two async/four maximum blocking threads without a server cgroup cap. Internal-network published ports were unreachable; the test-only stdlib relay forwards host loopback to the exact labelled DB container through docker-exec. **HTTP latency includes this relay cost.**

| Workload | Samples + warmup | RSS / process peak | p50 / p95 / p99 | Recall@10 / MRR@10 |
| --- | --- | --- | --- | --- |
| Standalone keyword, 1200 facts | 100 + 5 | 12.1 / 12.2 MiB | 74.5 / 115.8 / 270.0 ms | 1.00 / 1.00 |
| Compatibility keyword, 1200 facts | 100 + 5 | 10.1 / 10.1 MiB | 61.5 / 75.0 / 229.8 ms | 1.00 / 1.00 |
| Local MiniLM, four English facts/one repeated paraphrase | 10 + 5 | 106.4 / 188.7 MiB | 118.1 / 141.8 / 141.8 ms | 1.00 / 1.00 |

DB measured separately: Docker reports `80.99MiB / 512MiB`; Redis INFO reports RSS 93.2MiB and allocator peak 59.7MiB. Docker usage and Redis RSS have different accounting. All three server modes consumed 0.00 CPU seconds in their two-second idle sample at /proc counter resolution; this is not a long idle test.

A 32-concurrent/100-request read burst admitted 26 and rejected 74 with explicit search-busy responses. No search timeout/error occurred in sequential samples. Host swap usage decreased by 12KiB during the final run; that host-wide change is not attributable to this server.

## Persistence and retrieval limits

- Standalone SIGKILL/restart preserves the exact episode. Compatibility committed retry preserves metadata and repeats no mock model call after SIGKILL.
- Killing the compatibility process after dispatch begins leaves a failed receipt and no episode; restart does not replay the uncertain model request. The verifier reads the actual `jobs.sqlite` after stopping its worker; `worker.sqlite` is only the exclusive worker lock.
- Stopping only the isolated DB yields error status. Starting it restores the existing server connection. AOF restart preserves standalone/compatibility records and does not replay mock requests.
- The source-function/prepared/destructive integration cases remain distinct from live process fault tests. Disk-full/OOM, query runtime rollback, external writers and destructive interruption phases are not certified.
- The actual Python driver and hybrid edge-search path run in the isolated image with the observed edge-read patch and supplied mock vectors. Python finds the final vector outlier beyond the Rust candidate window; Rust compatibility misses it. This is concrete retrieval disagreement, not numerical provider parity. Full extraction, fuzzy resolution, summary/type/contradiction processing and actual MCP-wrapper parity are still unverified.
- Python SDK timing uses hybrid fulltext/vector queries inside the DB one-CPU cgroup; Rust keyword timing is HTTP plus relay and disables query embeddings. Their timing/RSS numbers do **not** establish a speed or resource ratio between equivalent servers.
- The local model test uses the pinned English MiniLM weights and actual CPU inference. Korean/multilingual quality, 256-token cost, growing corpus behavior and real LLM/Gemini comparisons remain untested.

## Reproduction and evidence

Build Linux ARM64 release executables with `Dockerfile.pi-validation`; transfer only executables, checked-in fixture/source files and the explicitly prepared local model to a fresh private synthetic root. Follow [Pi isolation plan](pi-validation.md). For this host use the relay overlay `compose.compat-pi-relay.yaml`, then `scripts/isolated-db-relay.py`; inspect the exact container labels/mount/entrypoint first. Run binaries with --include-ignored --test-threads=1 and explicit test DB/model variables, then `scripts/pi-synthetic-validation.py --test-root <owned-root>` with a clean environment.

The optional Python SDK baseline needs the observed edge patch copied read-only into the test container /tmp after its last restart, then `python -S scripts/python-pi-retrieval.py` with explicitly cleared environment and the synthetic marker. Never import the operational sitecustomize or start the application entrypoint. It is search-only and creates indexes solely in its synthetic graph.

Evidence: [benchmark JSON](pi-20261002/benchmark.json), [artifact hashes](pi-20261002/artifact-hashes.json), [Python retrieval](pi-20261002/python-retrieval.json), [results categories](results.json). Model/DB/journal files and executables are excluded from Git. Owned Mac/Pi test containers, network, relay, private data/model/journal directory and the failed fixture temporary directory were removed after preserving this synthetic evidence. Operational container remained healthy; its start time predates this test. No production restore/cutover/rollback was executed.
