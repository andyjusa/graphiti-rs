# Source excerpt: hermes_main.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def _clear_graph_group_aware(
    group_ids: list[str] | None = None,
) -> upstream.SuccessResponse | upstream.ErrorResponse:
    """Clear exactly one explicitly supplied writable group."""

    service = upstream.graphiti_service
    if service is None:
        return upstream.ErrorResponse(error="Graphiti service not initialized")
    try:
        group_id = require_exactly_one_group(group_ids, validate_group_id)
        ensure_writable_group(group_id, READ_ONLY_GROUP_IDS)
        client = await service.get_client()
        base_driver = client.driver
        driver = driver_for_group(base_driver, group_id)
        await upstream.clear_data(driver, group_ids=[group_id])
        return upstream.SuccessResponse(
            message=f"Graph data cleared successfully for group ID: {group_id}"
        )
    except GroupScopeViolation as error:
        upstream.logger.error("Graph clear group scope violation: %s", error)
        return upstream.ErrorResponse(error=f"Group scope violation: {error}")
    except Exception as error:
        upstream.logger.error("Scoped graph clear failed: %s", type(error).__name__)
        return upstream.ErrorResponse(error="Scoped graph clear failed")
