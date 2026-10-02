# Operational Graphiti replacement

The active objective is to replace the running Python Graphiti service with the lightweight Rust server on Raspberry Pi while retaining its existing memories, ten-tool MCP contract, authorization and ingestion/search behavior. Optional local embeddings remain available for independent new embedding spaces; existing Gemini vectors cannot silently change space.

Earlier synthetic preview work is evidence toward this objective, not completion. Every item below needs authoritative runtime/source evidence before marking the replacement complete.

- [x] Wire the observed staged ingestion into the worker: source-specific node extraction, prior same-source episodes, deterministic/fuzzy/model node resolution, edge extraction, duplicate/contradiction discovery, preserved historical provenance and incremental summaries.
- [ ] Preserve configured entity classifications and the deployed MCP relation profile. Compare the actual patched Python add_episode pipeline with Rust on identical synthetic inputs/mock responses, including multi-episode duplicates, aliases, contradictory and historical facts, Korean/message/JSON inputs, summary updates, failure and restart.
- [ ] Enable reviewed operational provider/storage configuration after the staged pipeline passes. Preserve bounded jobs/timeouts, explicit credential files, no response-body secret logging and durable accepted-job reconciliation.
- [x] Restore an initial original-memory backup privately on the Pi with the same DB module; audit typed records, UUIDs, times, provenance and native vector dimensions in all 63 policy groups. Explicitly classify historical scope exclusions; preserve the nine outside-policy graphs and every opaque copy. Retain the original backup outside Git/CI. See restore-audit.md.
- [ ] Compare original-memory MCP read results against the recorded Python reader and measure Rust/DB resources on restored data. The maintenance audit is structural evidence, not this result-equivalence gate.
- [ ] Verify bounded real provider generation/embedding and multilingual retrieval against the deployed profile. Measure Rust and DB resource use on the actual Pi with representative restored data.
- [ ] Prepare and exercise a separate authenticated test gateway/session; verify all tool wrapping, policy and reconnect behavior and a concrete rollback path.
- [ ] Drain/freeze the old ingestion queue, reconcile accepted jobs, switch the operational upstream to the verified Rust service, and verify reads plus a controlled write/read after the switch. Keep the prior service and backup available for rollback.
- [ ] Verify the active clients actually use Rust and the old Python ingestion service no longer receives new work. Publish the final repository/PR state and runtime evidence; only then complete the goal.

Source baseline: observed graphiti-core0.28.2 and user-owned stable UUID/group/edge/JSON/Gemini patches in observed-production.json. Live state must be rechecked before operational steps. Existing preview restrictions are deliberate staging boundaries; they must not be mistaken for the final desired state.

Current external provider condition: the observed Gemini key can read model metadata, but synthetic embedding calls receive HTTP 402 RESOURCE_EXHAUSTED for depleted Prepay credit. Both original Python and Rust fail at that stage. No more embedding retries or operational cutover until the account is restored or an explicitly supplied working key for the same model is available; independent reader/gateway preparation can continue. See restore-audit.md for sanitized evidence and the official API error reference.
