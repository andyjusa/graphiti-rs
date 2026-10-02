# Global vector retrieval fix — 2026-10-02

**The previously observed vector outlier miss is fixed. Python-server replacement remains no-go for the separate pipeline/restore/provider/gateway gates.**

Source `48cdbd84a04a6f5f23eb3e955f8a46be7f9523d6` (server Rust source introduced in `8f68eaa5b62d16b7424463d8757214d77fc8c66f`). Linux ARM64 release, both optional features, actual Raspberry Pi4/aarch64/3795MiB RAM. [Artifact hashes](pi-vector-fix-20261002/artifact-hashes.json) tie the tested executables to this source.

## Change

Compatibility fact/node queries calculate similarity across all supported vectors in the requested group, then return only the bounded top candidates. They no longer truncate by UUID before calculating similarity. Type/group filtering precedes ranking. Relationship-first queries preserve the observed indexed-edge workaround. The score now follows the observed Python Falkor expression `(2 - vec.cosineDistance(...))/2` with strict minimum 0.6; the old Rust path incorrectly applied 0.6 to raw cosine. The DB-provided semantic order enters RRF directly. Keyword retrieval remains a bounded substring search and is not Python BM25.

Standalone ingestion stores native `vecf32` values alongside the existing JSON DTO and marks newly written records with `embedding_version=1`. Semantic retrieval ranks all eligible native vectors before the candidate bound, with group and half-open validity predicates. Existing JSON-only facts remain readable: a read-only 128-row keyset scan maintains a bounded best-candidate list. The semantic operation has a 30-second overall deadline in addition to the existing five-second DB/six-second command deadlines. A failed or timed-out scan returns an error rather than silently returning partial results. It performs no implicit backfill. Older corpora may pay the page-scan cost; native vector storage adds disk/DB memory alongside the retained JSON representation. Result bounds do not cap total DB scan work, sorting memory or data volume.

## Verification

- Local fmt, all-target/all-feature clippy and 26 nonignored tests pass.
- [CI on this source](https://github.com/andyjusa/graphiti-rs/actions/runs/36979109286) passes **45 distinct executable test names** and ARM64 default/local/compatibility release builds. CI module is 4.14.11.
- All **45 release tests pass on actual Pi module 4.16.6**, using the observed image/Redis8.2.3/module hash. [Test log](pi-vector-fix-20261002/tests.txt). Three new regressions cover global compatibility fact/node retrieval and normalized threshold/type filtering; native/old-JSON retrieval with group/time/provenance/zero-vector/dimension checks; and standalone semantic recall outside the recent window. The latter uses a keyword-neutral mock query so incidental single-letter substring matches do not obscure semantic recall.
- The 1200-fact MCP fixture now finds the final compatibility vector fact and node at rank 1, beyond the former 200-candidate UUID window. The text/identity corpus hash matches the earlier Python baseline; fact vectors remain identical. [Benchmark](pi-vector-fix-20261002/benchmark.json). The earlier [miss](pi-20261002/benchmark.json) is preserved as historical evidence.
- The actual Python0.28.2 hybrid edge-search path was rerun inside the isolated image with the observed edge patch and supplied vectors. [New Python result](pi-vector-fix-20261002/python-retrieval.json): outlier recall@10=1, with exactly the same returned UUID list as Rust; the text/identity corpus hashes match. This closes the specific outlier disagreement, not complete ingestion/MCP/provider or BM25 parity. Python's timing includes the hybrid SDK path inside the one-CPU DB cgroup; Rust's keyword profile uses HTTP/relay without query embeddings, so no equivalent-server speed ratio is inferred.
- Standalone finds its old target first with 201 newer decoys, both with native vectors and after the fixture explicitly removes native fields to simulate the original JSON-only schema. These use loopback 2-dimensional mock embeddings, not an actual model quality benchmark.
- Actual local English MiniLM CPU inference and the four-fact repeated-paraphrase smoke still pass. Standalone SIGKILL restart, compatibility committed/uncertain-dispatch recovery, isolated DB outage/reconnection and AOF restart pass with no mock-provider replay.

## Bounded synthetic measurements

DB has one CPU/512MiB, and the server uses two async/four maximum blocking threads without its own cgroup cap. HTTP timings include the labelled docker-exec loopback relay. These single runs do not certify a production budget or an equivalent Python/Rust speed ratio.

| Workload | Samples + warmup | RSS / process peak | p50 / p95 / p99 | Recall@10 / MRR@10 |
| --- | --- | --- | --- | --- |
| Standalone keyword, 1200 facts | 100 + 5 | 11.6 / 11.8 MiB | 72.2 / 116.2 / 263.5 ms | 1.00 / 1.00 |
| Compatibility keyword, 1200 facts | 100 + 5 | 10.0 / 10.0 MiB | 60.4 / 72.6 / 227.9 ms | 1.00 / 1.00 |
| Local MiniLM, four English facts | 10 + 5 | 106.7 / 188.9 MiB | 110.5 / 129.2 / 129.2 ms | 1.00 / 1.00 |

Standalone native outlier: 105.3ms, 12.2MiB RSS. Original-JSON fallback: 207.1ms, 12.2MiB RSS. Each is one mock-vector sample over 202 facts, not a distribution or multilingual quality evaluation. The 32-concurrent/100-request burst admits 30 and explicitly rejects 70 as busy.

DB accounting is separate: Docker reports `60.23MiB / 512MiB`; Redis INFO reports RSS 72.8MiB and allocator peak 40.5MiB. Host-wide swap usage stays unchanged during the harness. Schema breadth, unsupported/mixed vector types, external writers and growing-corpus timeout behavior still need broader acceptance testing.

[Reproduction/isolation](pi-validation.md), [implementation](implementation.md) and [remaining gates](verification.md) apply. No operational data, credentials, application entrypoint, gateway or route was used for the synthetic harness; no original-data restore or actual cutover was performed.

After preserving the synthetic evidence, only the labelled test container/network, verified relay PID and private synthetic root were removed. [Cleanup record](pi-vector-fix-20261002/cleanup.json) confirms the operational container is healthy with its original pre-test start time.
