# Source excerpt: hermes_main.py
# Graphiti core portions: Copyright 2024, Zep Software, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def _delete_entity_edge_group_aware(
    uuid: str,
    group_id: str,
) -> upstream.SuccessResponse | upstream.ErrorResponse:
    """Delete one entity edge from exactly one writable, explicit group."""

    service = upstream.graphiti_service
    if service is None:
        return upstream.ErrorResponse(error="Graphiti service not initialized")
    try:
        validate_group_id(group_id)
        ensure_writable_group(group_id, READ_ONLY_GROUP_IDS)
        client = await service.get_client()
        driver, entity_edge = await lookup_group_scoped(
            client,
            group_id,
            uuid,
            EntityEdge.get_by_uuid,
        )
        await entity_edge.delete(driver)
        return upstream.SuccessResponse(
            message=f"Entity edge with UUID {uuid} deleted successfully"
        )
    except GroupScopeViolation as error:
        upstream.logger.error("Entity edge delete group scope violation: %s", error)
        return upstream.ErrorResponse(error=f"Group scope violation: {error}")
    except Exception as error:
        upstream.logger.error("Exact entity edge delete failed: %s", type(error).__name__)
        return upstream.ErrorResponse(error="Exact entity edge delete failed")
