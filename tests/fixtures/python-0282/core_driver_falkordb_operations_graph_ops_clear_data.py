# Source excerpt: core/driver/falkordb/operations/graph_ops.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def clear_data(
    self,
    executor: QueryExecutor,
    group_ids: list[str] | None = None,
) -> None:
    if group_ids is None:
        await executor.execute_query('MATCH (n) DETACH DELETE n')
    else:
        # FalkorDB: iterate labels individually
        for label in ['Entity', 'Episodic', 'Community']:
            await executor.execute_query(
                f"""
                MATCH (n:{label})
                WHERE n.group_id IN $group_ids
                DETACH DELETE n
                """,
                group_ids=group_ids,
            )
