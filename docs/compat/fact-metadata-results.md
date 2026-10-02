# Existing fact metadata compatibility

The observed SDK treats exact reuse and generic model-based reuse differently. Exact matching returns the existing relation with its attributes. The generic model branch resolves the duplicate but sets its structured attributes to an empty dictionary when no custom relation model is configured. Historical invalidation retains the old relation's attributes.

Rust's resolution already followed that distinction, but durable planning merged the old attributes back into the result. The commit query also added properties to the existing relation instead of replacing them with the complete planned map. Consequently, a generic model duplicate retained metadata that the actual SDK removed.

## Counterexample and correction

A fixed synthetic four-episode cohort seeds two flat, nonsecret legacy properties (`fixture_note` and `fixture_weight`) on two existing employment relations after episode zero. In the next episode, one relation is reused through two model-resolved paraphrases and the other through the exact-text path. A later episode invalidates the second relation.

Before correction, a real SDK replay on the Pi fails at episode one: the SDK's model-resolved relation has `{}` attributes, while Rust retains both old properties. Ordered references and all other canonical fields agree. The exact-text control retains its two properties in the SDK, confirming this is a branch-specific effect rather than a blanket metadata reset.

After verifying the unchanged before-state, `plan_observed` now retains the staged payload's authoritative fact attributes. Complete relationship properties are assigned from the durable planned map, allowing cleared attributes to disappear physically. The exact-reuse and invalidation payloads already carry the old values, so those paths preserve them. Legacy direct planning continues to merge existing attributes into its complete map. Node planning, DTOs, UUIDs, endpoint checks, provenance, embedding generation, CAS predicates and journal schema are unchanged.

The corrected ARM64 release fixture and actual Graphiti 0.28.2/Python 3.11.2 SDK match all four canonical graphs, including summaries, classifications, attributes, provenance, validity and exact expiry instants. The SDK wrapper independently asserts generic-model clearing, exact preservation and later invalidation preservation. Twenty-one supplied responses, real Pydantic schemas, eight matching candidate fact sets, ordered references `6, 3, 7, 4` and all 56 stored 3072-dimensional basis-vector records agree. Independent UUID/ranking ties still require fact-index rebasing; real model choices and fresh ranking quality are not certified.

Both the failing baseline and corrected run retain prospective manifests, exact source patches/snapshots, uploaded-file hashes and append-only execution/results. Only sanitized evidence is committed in [the summary](pi-fact-metadata-20261003/summary.json). The fixture uses owned synthetic data, not an original-memory mutation.

## Validation and limits

Local formatting, all-target/all-feature Clippy, 31 non-ignored tests and all 15 isolated compatibility DB tests pass. Those tests include direct legacy merge/CAS/provenance operations, both overlapping extraction orders and prepared-journal fault recovery. The new seeded metadata test also passes on the actual Pi with the original FalkorDB 4.16.6 module; local DB tests use 4.14.11.

Pi runs use owned empty DBs/internal networks, read-only container roots, explicit bounded `/data` and `/tmp` memory and owned loopback relays. Python uses a cleared environment, `-S`, disabled telemetry and the original source-pinned edge-query patch. Providers are supplied loopback mocks. Exact task resources are removed, original service/gateway/restore identities/start times and initial backup/audit hashes are unchanged, and no real provider or operating write is made.

For an exclusively owned disposable DB:

```sh
COMPAT_TEST_DB_URL=redis://127.0.0.1:16379 cargo test --locked --all-features --test compat interacting_relations::existing_metadata_exact_model_reuse_and_invalidation -- --ignored --exact --nocapture
```

`COMPAT_RELATION_REPORT` writes a new private report. The pinned SDK replay requires `python-pi-fact-metadata.py`, the existing relation/pipeline helpers, original edge patch and report in its owned container. The wrapper seeds exactly the same synthetic legacy properties before the unchanged canonical evaluator; it does not alter SDK resolution/persistence. Task ownership and preservation checks belong to private orchestration.

This verifies two flat legacy properties across the recorded generic-model, exact and invalidation paths. Custom relation models and arbitrary metadata schemas are outside this recorded comparison. Larger overlap/fault cases, real provider/new-query quality, fresh OAuth and operating cutover remain pending. This is a further compatibility gate, not completion of the operating replacement.
