# Interacting relation compatibility

The staged Rust pipeline previously built each episode's `entity_edges` from its unique fact records. The observed Python SDK keeps one reference for every resolved edge and then every invalidation, including repeated UUIDs. Two differently worded facts resolved to the same existing relation therefore produced three references in Python and only two in Rust. An actual SDK replay on the Pi reproduced this discrepancy before correction.

Rust now preserves reference order and multiplicity while storing a bounded set of unique fact records. The prepared journal still rejects unknown or missing fact targets, more than 256 unique facts and excessive reference lists. The reference bound is derived from the existing extraction/decision limits: 128 resolved edges plus up to 256 invalidations for each. Historical receipt replay still avoids new model calls.

## Bounded cohort

Four synthetic message/JSON/text episodes cover six persistent entities and cumulative fact counts `6, 6, 9, 11`:

| Episode | Cases | Ordered references |
| --- | --- | --- |
| Message | Employment, mentorship, partnership, volunteering and a self-loop; normalized extraction duplicate, missing endpoint and blank fact filtering | 6 |
| JSON | Alias resolves to the original entity; two paraphrases reuse one relation, plus exact reuse of another | 3, including one repeated UUID |
| Text | Three new relations invalidate older relations; another extracted fact reuses a relation also selected for invalidation | 7, including one UUID in both resolved and invalidation references |
| Message | Two relations from earlier valid times expire against later relations; exact mentorship and self-loop reuse | 4 |

The actual pinned Graphiti 0.28.2 SDK on Python 3.11.2 invokes `Graphiti.add_episode` with twenty-one supplied model responses and 3072-dimensional basis vectors. All four canonical snapshots match, including summaries, classifications, attributes, creation/validity times, provenance and exact `expired_at` instants. The comparison separately checks the ordered fact identities in every episode reference, stable entity UUIDs, real Pydantic schemas, candidate fact sets and all 56 stored node/fact vector records across snapshots. The Rust fixture verifies same-UUID receipt replay after every episode without another model request.

For the third episode, the actual SDK leaves the reused employment relation active with the current episode's support, while the other two invalidated relations expire and retain historical support. This is a result of this specific replay, not a general assertion about duplicate-write ordering or exhaustive overlapping invalidation behavior. The intermediate assumption that an invalidation copy always wins was rejected by the real SDK comparison.

Fresh Rust/Python graphs allocate independent UUIDs, and tied candidates can appear in a different order. The replay requires the same candidate fact sets, then rebases supplied decision indices to those same facts. It does not certify identical fresh candidate ordering or real model choices. The actual SDK executes resolution and persistence; model responses, embeddings and the clock are supplied. The fixture refuses unexpected cross-encoder calls. Sanitized counts, hashes and preservation checks are in [the evidence](pi-interacting-relations-20261003/summary.json).

## Isolation and validation

All 31 non-ignored all-feature tests, all 13 isolated compatibility DB tests, formatting and all-target/all-feature Clippy pass locally. The new fixture also passes as an ARM64 release binary on the actual Pi, and the actual SDK replay matches the four resulting graphs. Local tests use FalkorDB 4.14.11; the Pi uses the original 4.16.6 module.

The Pi comparison runs in an owned internal Docker network with a fresh DB, read-only container root, explicit bounded `/data` and `/tmp` memory and an owned loopback relay. Python runs with an empty environment, `-S`, disabled telemetry and the observed source-hash-pinned edge-query patch. Rust model endpoints are loopback mocks. No original memories, credentials or operational sockets are mounted. Exact owned container/network/relay cleanup, original service/gateway/restore identities and start times, and the initial backup/restored audit hashes are rechecked. Original services and routing remain unchanged.

For an exclusively owned disposable DB:

```sh
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 cargo test --locked --all-features --test compat interacting_relations::interacting_reuse_invalidation_alias_self_loop_and_historical_relations -- --ignored --exact --nocapture
```

`COMPAT_RELATION_REPORT` writes a new owner-private replay file. The Python replay requires that file as `/tmp/relation-rust.json`, the existing `python-pi-pipeline.py` helper and original edge patch inside the pinned disposable SDK container. Ownership/resource checks belong to the private Pi orchestration; the replay script is not an operational ingestion command.

A subsequent extraction-order counterexample and correction are recorded in [overlap-order-results.md](overlap-order-results.md). Both fixed orders now match the actual SDK.

This closes this bounded interacting-relation cohort. Wider cross-SDK fault/restart and overlapping-write cases, real provider/new-query quality, fresh OAuth and operational cutover remain pending.
