# Source excerpt: core/driver/falkordb/operations/search_ops.py
# Graphiti core portions: Copyright 2024, Zep Software, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def node_distance_reranker(
    self,
    executor: QueryExecutor,
    node_uuids: list[str],
    center_node_uuid: str,
    min_score: float = 0,
) -> list[EntityNode]:
    filtered_uuids = [u for u in node_uuids if u != center_node_uuid]
    scores: dict[str, float] = {center_node_uuid: 0.0}

    cypher = """
    UNWIND $node_uuids AS node_uuid
    MATCH (center:Entity {uuid: $center_uuid})-[:RELATES_TO]-(n:Entity {uuid: node_uuid})
    RETURN 1 AS score, node_uuid AS uuid
    """

    results, _, _ = await executor.execute_query(
        cypher,
        node_uuids=filtered_uuids,
        center_uuid=center_node_uuid,
    )

    for result in results:
        scores[result['uuid']] = result['score']

    for uuid in filtered_uuids:
        if uuid not in scores:
            scores[uuid] = float('inf')

    filtered_uuids.sort(key=lambda cur_uuid: scores[cur_uuid])

    if center_node_uuid in node_uuids:
        scores[center_node_uuid] = 0.1
        filtered_uuids = [center_node_uuid] + filtered_uuids

    reranked_uuids = [u for u in filtered_uuids if (1 / scores[u]) >= min_score]

    if not reranked_uuids:
        return []

    get_query = """
        MATCH (n:Entity)
        WHERE n.uuid IN $uuids
        RETURN
        """ + get_entity_node_return_query(GraphProvider.FALKORDB)

    records, _, _ = await executor.execute_query(get_query, uuids=reranked_uuids)

    node_map = {r['uuid']: entity_node_from_record(r) for r in records}
    return [node_map[u] for u in reranked_uuids if u in node_map]
