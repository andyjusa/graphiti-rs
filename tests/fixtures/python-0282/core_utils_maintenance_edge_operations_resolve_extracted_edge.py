# Source excerpt: core/utils/maintenance/edge_operations.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def resolve_extracted_edge(
    llm_client: LLMClient,
    extracted_edge: EntityEdge,
    related_edges: list[EntityEdge],
    existing_edges: list[EntityEdge],
    episode: EpisodicNode,
    edge_type_candidates: dict[str, type[BaseModel]] | None = None,
) -> tuple[EntityEdge, list[EntityEdge], list[EntityEdge]]:
    """Resolve an extracted edge against existing graph context.

    Parameters
    ----------
    llm_client : LLMClient
        Client used to invoke the LLM for deduplication and attribute extraction.
    extracted_edge : EntityEdge
        Newly extracted edge whose canonical representation is being resolved.
    related_edges : list[EntityEdge]
        Candidate edges with identical endpoints used for duplicate detection.
    existing_edges : list[EntityEdge]
        Broader set of edges evaluated for contradiction / invalidation.
    episode : EpisodicNode
        Episode providing content context when extracting edge attributes.
    edge_type_candidates : dict[str, type[BaseModel]] | None
        Custom edge types permitted for the current source/target signature.

    Returns
    -------
    tuple[EntityEdge, list[EntityEdge], list[EntityEdge]]
        The resolved edge, any duplicates, and edges to invalidate.
    """
    if len(related_edges) == 0 and len(existing_edges) == 0:
        # Still extract custom attributes even when no dedup/invalidation is needed
        edge_model = (
            edge_type_candidates.get(extracted_edge.name) if edge_type_candidates else None
        )
        if edge_model is not None and len(edge_model.model_fields) != 0:
            edge_attributes_context = {
                'fact': extracted_edge.fact,
                'reference_time': episode.valid_at if episode is not None else None,
                'existing_attributes': extracted_edge.attributes,
            }
            edge_attributes_response = await llm_client.generate_response(
                prompt_library.extract_edges.extract_attributes(edge_attributes_context),
                response_model=edge_model,  # type: ignore
                model_size=ModelSize.small,
                prompt_name='extract_edges.extract_attributes',
            )
            extracted_edge.attributes = edge_attributes_response

        return extracted_edge, [], []

    # Fast path: if the fact text and endpoints already exist verbatim, reuse the matching edge.
    normalized_fact = _normalize_string_exact(extracted_edge.fact)
    for edge in related_edges:
        if (
            edge.source_node_uuid == extracted_edge.source_node_uuid
            and edge.target_node_uuid == extracted_edge.target_node_uuid
            and _normalize_string_exact(edge.fact) == normalized_fact
        ):
            resolved = edge
            if episode is not None and episode.uuid not in resolved.episodes:
                resolved.episodes.append(episode.uuid)
            return resolved, [], []

    start = time()

    # Prepare context for LLM with continuous indexing
    related_edges_context = [{'idx': i, 'fact': edge.fact} for i, edge in enumerate(related_edges)]

    # Invalidation candidates start where duplicate candidates end
    invalidation_idx_offset = len(related_edges)
    invalidation_edge_candidates_context = [
        {'idx': invalidation_idx_offset + i, 'fact': existing_edge.fact}
        for i, existing_edge in enumerate(existing_edges)
    ]

    context = {
        'existing_edges': related_edges_context,
        'new_edge': extracted_edge.fact,
        'edge_invalidation_candidates': invalidation_edge_candidates_context,
    }

    if related_edges or existing_edges:
        logger.debug(
            'Resolving edge: sent %d EXISTING FACTS%s and %d INVALIDATION CANDIDATES%s',
            len(related_edges),
            f' (idx 0-{len(related_edges) - 1})' if related_edges else '',
            len(existing_edges),
            f' (idx {invalidation_idx_offset}-{invalidation_idx_offset + len(existing_edges) - 1})'
            if existing_edges
            else '',
        )

    llm_response = await llm_client.generate_response(
        prompt_library.dedupe_edges.resolve_edge(context),
        response_model=EdgeDuplicate,
        model_size=ModelSize.small,
        prompt_name='dedupe_edges.resolve_edge',
    )
    response_object = EdgeDuplicate(**llm_response)
    duplicate_facts = response_object.duplicate_facts

    # Validate duplicate_facts are in valid range for EXISTING FACTS
    invalid_duplicates = [i for i in duplicate_facts if i < 0 or i >= len(related_edges)]
    if invalid_duplicates:
        logger.warning(
            'LLM returned invalid duplicate_facts idx values %s (valid range: 0-%d for EXISTING FACTS)',
            invalid_duplicates,
            len(related_edges) - 1,
        )

    duplicate_fact_ids: list[int] = [i for i in duplicate_facts if 0 <= i < len(related_edges)]

    resolved_edge = extracted_edge
    for duplicate_fact_id in duplicate_fact_ids:
        resolved_edge = related_edges[duplicate_fact_id]
        break

    if duplicate_fact_ids and episode is not None:
        resolved_edge.episodes.append(episode.uuid)

    # Process contradicted facts (continuous indexing across both lists)
    contradicted_facts: list[int] = response_object.contradicted_facts
    invalidation_candidates: list[EntityEdge] = []

    # Only process contradictions if there are edges to check against
    if related_edges or existing_edges:
        max_valid_idx = len(related_edges) + len(existing_edges) - 1
        invalid_contradictions = [i for i in contradicted_facts if i < 0 or i > max_valid_idx]
        if invalid_contradictions:
            logger.warning(
                'LLM returned invalid contradicted_facts idx values %s (valid range: 0-%d)',
                invalid_contradictions,
                max_valid_idx,
            )

        # Split contradicted facts into those from related_edges vs existing_edges based on offset
        for idx in contradicted_facts:
            if 0 <= idx < len(related_edges):
                # From EXISTING FACTS (duplicate candidates)
                invalidation_candidates.append(related_edges[idx])
            elif invalidation_idx_offset <= idx <= max_valid_idx:
                # From FACT INVALIDATION CANDIDATES (adjust index by offset)
                invalidation_candidates.append(existing_edges[idx - invalidation_idx_offset])

    # Only extract structured attributes if the edge's relation_type matches an allowed custom type
    # AND the edge model exists for this node pair signature
    edge_model = edge_type_candidates.get(resolved_edge.name) if edge_type_candidates else None
    if edge_model is not None and len(edge_model.model_fields) != 0:
        edge_attributes_context = {
            'fact': resolved_edge.fact,
            'reference_time': episode.valid_at if episode is not None else None,
            'existing_attributes': resolved_edge.attributes,
        }

        edge_attributes_response = await llm_client.generate_response(
            prompt_library.extract_edges.extract_attributes(edge_attributes_context),
            response_model=edge_model,  # type: ignore
            model_size=ModelSize.small,
            prompt_name='extract_edges.extract_attributes',
        )

        resolved_edge.attributes = edge_attributes_response
    else:
        resolved_edge.attributes = {}

    end = time()
    logger.debug(
        f'Resolved Edge: {extracted_edge.uuid} -> {resolved_edge.uuid}, in {(end - start) * 1000} ms'
    )

    now = utc_now()

    if resolved_edge.invalid_at and not resolved_edge.expired_at:
        resolved_edge.expired_at = now

    # Determine if the new_edge needs to be expired
    if resolved_edge.expired_at is None:
        invalidation_candidates.sort(key=lambda c: (c.valid_at is None, ensure_utc(c.valid_at)))
        for candidate in invalidation_candidates:
            candidate_valid_at_utc = ensure_utc(candidate.valid_at)
            resolved_edge_valid_at_utc = ensure_utc(resolved_edge.valid_at)
            if (
                candidate_valid_at_utc is not None
                and resolved_edge_valid_at_utc is not None
                and candidate_valid_at_utc > resolved_edge_valid_at_utc
            ):
                # Expire new edge since we have information about more recent events
                resolved_edge.invalid_at = candidate.valid_at
                resolved_edge.expired_at = now
                break

    # Determine which contradictory edges need to be expired
    invalidated_edges: list[EntityEdge] = resolve_edge_contradictions(
        resolved_edge, invalidation_candidates
    )
    duplicate_edges: list[EntityEdge] = [related_edges[idx] for idx in duplicate_fact_ids]

    return resolved_edge, invalidated_edges, duplicate_edges
