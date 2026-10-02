# Operational Graphiti replacement

The active objective is to replace the running Python Graphiti service with the lightweight Rust server on Raspberry Pi while retaining its existing memories, ten-tool MCP contract, authorization and ingestion/search behavior. Optional local embeddings remain available for independent new embedding spaces; existing Gemini vectors cannot silently change space.

Earlier synthetic preview work is evidence toward this objective, not completion. Every item below needs authoritative runtime/source evidence before marking the replacement complete.

- [x] Wire the observed staged ingestion into the worker: source-specific node extraction, prior same-source episodes, deterministic/fuzzy/model node resolution, edge extraction, duplicate/contradiction discovery, preserved historical provenance and incremental summaries.
- [ ] Preserve configured entity classifications and the deployed MCP relation profile. Compare the actual patched Python add_episode pipeline with Rust on identical synthetic inputs/mock responses, including multi-episode duplicates, aliases, contradictory and historical facts, Korean/message/JSON inputs, summary updates, failure and restart.
- [ ] Enable reviewed operational provider/storage configuration after the staged pipeline passes. Preserve bounded jobs/timeouts, explicit credential files, no response-body secret logging and durable accepted-job reconciliation.
- [ ] Restore an original-memory backup privately on the Pi with the same DB module; verify original schema, groups, UUIDs, times, provenance, vector dimensions and read results. Retain the original backup outside Git/CI.
- [ ] Verify bounded real provider generation/embedding and multilingual retrieval against the deployed profile. Measure Rust and DB resource use on the actual Pi with representative restored data.
- [ ] Prepare and exercise a separate authenticated test gateway/session; verify all tool wrapping, policy and reconnect behavior and a concrete rollback path.
- [ ] Drain/freeze the old ingestion queue, reconcile accepted jobs, switch the operational upstream to the verified Rust service, and verify reads plus a controlled write/read after the switch. Keep the prior service and backup available for rollback.
- [ ] Verify the active clients actually use Rust and the old Python ingestion service no longer receives new work. Publish the final repository/PR state and runtime evidence; only then complete the goal.

Source baseline: observed graphiti-core0.28.2 and user-owned stable UUID/group/edge/JSON/Gemini patches in observed-production.json. Live state must be rechecked before operational steps. Existing preview restrictions are deliberate staging boundaries; they must not be mistaken for the final desired state.
