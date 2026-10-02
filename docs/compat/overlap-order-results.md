# Overlapping reuse and invalidation order

An episode can contain a paraphrase reusing an existing relation and another fact invalidating that same relation. The original interacting-relation cohort placed reuse last and matched Python. Moving only that reused extraction to the beginning exposed a new mismatch on the actual Pi: Rust expired the relation and omitted the current episode's support, while the observed SDK kept it active with that support. The episode reference counts and order already matched. This counterexample demonstrates why one passing order did not certify the overlapping update.

Rust now accumulates resolved and invalidated fact payloads separately, retains the first copy of each UUID within each phase and uses an invalidation payload only when there is no resolved payload for that UUID. Ordered repeated episode references remain independent from the bounded set of unique stored facts. The unique-fact union remains capped at 256; CAS, durable journal reconciliation, summaries and vector validation are unchanged.

## Native writer evidence and scope

A separate provider-free probe executes the exact deployed SDK Falkor bulk-save query on six fresh synthetic graphs. Payload A is active and includes current support; B expires at September 10 and C at September 12, both with historical support. Every case has one physical relation and a checked 3072-dimensional basis vector.

| Relation before write | Payload order | Stored copy |
| --- | --- | --- |
| Absent | A, B | A |
| Absent | B, A | B |
| Absent | A, B, C | B |
| Existing | A, B | A |
| Existing | B, A | B |
| Existing | A, B, C | A |

The fresh three-copy result rejects a universal first-payload rule. The correction applies to the staged pipeline's reachable repeated UUIDs: each newly extracted fact receives its own fresh UUID, normalized extraction duplicates are filtered, and duplicate/invalidation candidates come from existing DB relations. Thus overlapping payloads for one UUID belong to existing relations. This is a source-derived invariant, not an exclusion chosen after a failing valid staged input. The six probes support the recorded existing-relation cases; they do not establish all query plans or arbitrary external bulk-write behavior.

## Paired staged verification

The corrected ARM64 release fixture runs both fixed extraction orders independently on the Pi, followed by real `Graphiti.add_episode` calls from the pinned Graphiti 0.28.2/Python 3.11.2 SDK. Each uses four message/JSON/text episodes, six entities, cumulative fact counts `6, 6, 9, 11`, and twenty-one supplied model responses. Both orders match all four canonical stored graphs, exact validity/expiry instants, summaries/classifications/attributes/provenance, ordered reference counts `6, 3, 7, 4`, stable identities and real Pydantic schemas. Eight candidate fact sets per order match; decisions are rebased to their same fact identities because fresh UUIDs and ranking ties can change candidate order. All 56 stored node/fact basis-vector records match in each order. Rust same-UUID receipts make no additional model calls.

The old implementation's reversed-order SDK failure is retained privately, separately from the six native probes and two corrected runs. Prospective manifests, exact patches/source snapshots, uploaded-file hashes, execution identities and append-only results are recorded before each core launch. One corrected run's extra protected-source hashes were supplemented after launch and verified against the immutable baseline; its original manifest is preserved. This limits the prospective coverage claim for that supplemental check, not the actual paired comparison.

Local formatting, all-target/all-feature Clippy, 31 non-ignored all-feature tests and all 14 isolated compatibility DB tests pass. The first local DB attempt failed because Docker's internal-only network did not publish the host port. The DB was confirmed alive by its own PING; adding a task-owned host bridge restored loopback access without restarting it. All fourteen tests then passed. The failure log is retained and is not treated as a source regression or parity evidence.

Sanitized counts, source/query/binary hashes, the native negative control and original-service preservation checks are in [the evidence](pi-overlap-order-20261003/summary.json).

## Isolation and remaining gates

Pi comparisons use the original FalkorDB 4.16.6 module in exclusively owned empty databases, internal networks, read-only container roots and explicit bounded memory for `/data` and `/tmp`. The SDK runs with a cleared environment, `-S`, disabled telemetry and mocks. The native writer probe makes no model calls; Rust/SDK ingestion calls are loopback mocks. No original-memory or credential mounts are used. Exact owned containers/networks/relays are removed, while original service/gateway/restore identities/start times and backup/audit hashes are preserved. Local disposable tests use 4.14.11 and a task-owned bridge for the loopback test port.

For an exclusively owned disposable DB:

```sh
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 cargo test --locked --all-features --test compat interacting_relations::overlap_order_reuse_before_invalidation -- --ignored --exact --nocapture
```

The earlier interacting-relation test supplies the opposite extraction order. Each can write a new private `COMPAT_RELATION_REPORT` for the existing pinned SDK replay script. Ownership checks and original-service preservation belong to the Pi orchestration; these are not operational ingestion commands.

This closes the two fixed overlapping-update orders, not exhaustive ingestion/fault parity. Larger overlapping candidate sets, cross-SDK failure/restart comparisons, real-provider/new-query quality, fresh OAuth and operating cutover remain pending. No paid model calls or operating switch occurred.
