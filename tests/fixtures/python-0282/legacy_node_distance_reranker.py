# Copyright 2024, Zep Software, Inc.; Apache-2.0.
async def node_distance_reranker(
    driver: GraphDriver,
    node_uuids: list[str],
    center_node_uuid: str,
    min_score: float = 0,
) -> tuple[list[str], list[float]]:
    if driver.search_interface:
        try:
            return await driver.search_interface.node_distance_reranker(
                driver, node_uuids, center_node_uuid, min_score
            )
        except NotImplementedError:
            pass

    # filter out node_uuid center node node uuid
    filtered_uuids = list(filter(lambda node_uuid: node_uuid != center_node_uuid, node_uuids))
    scores: dict[str, float] = {center_node_uuid: 0.0}

    query = """
    UNWIND $node_uuids AS node_uuid
    MATCH (center:Entity {uuid: $center_uuid})-[:RELATES_TO]-(n:Entity {uuid: node_uuid})
    RETURN 1 AS score, node_uuid AS uuid
    """
    if driver.provider == GraphProvider.KUZU:
        query = """
        UNWIND $node_uuids AS node_uuid
        MATCH (center:Entity {uuid: $center_uuid})-[:RELATES_TO]->(e:RelatesToNode_)-[:RELATES_TO]->(n:Entity {uuid: node_uuid})
        RETURN 1 AS score, node_uuid AS uuid
        """

    # Find the shortest path to center node
    results, header, _ = await driver.execute_query(
        query,
        node_uuids=filtered_uuids,
        center_uuid=center_node_uuid,
        routing_='r',
    )
    if driver.provider == GraphProvider.FALKORDB:
        results = [dict(zip(header, row, strict=True)) for row in results]

    for result in results:
        uuid = result['uuid']
        score = result['score']
        scores[uuid] = score

    for uuid in filtered_uuids:
        if uuid not in scores:
            scores[uuid] = float('inf')

    # rerank on shortest distance
    filtered_uuids.sort(key=lambda cur_uuid: scores[cur_uuid])

    # add back in filtered center uuid if it was filtered out
    if center_node_uuid in node_uuids:
        scores[center_node_uuid] = 0.1
        filtered_uuids = [center_node_uuid] + filtered_uuids

    return [uuid for uuid in filtered_uuids if (1 / scores[uuid]) >= min_score], [
        1 / scores[uuid] for uuid in filtered_uuids if (1 / scores[uuid]) >= min_score
    ]
