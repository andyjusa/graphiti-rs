# Source excerpt: core/edges.py
# Graphiti core portions: Copyright 2024, Zep Software, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def delete(self, driver: GraphDriver):
    if driver.graph_operations_interface:
        try:
            return await driver.graph_operations_interface.edge_delete(self, driver)
        except NotImplementedError:
            pass

    if driver.provider == GraphProvider.KUZU:
        await driver.execute_query(
            """
            MATCH (n)-[e:MENTIONS|HAS_MEMBER {uuid: $uuid}]->(m)
            DELETE e
            """,
            uuid=self.uuid,
        )
        await driver.execute_query(
            """
            MATCH (e:RelatesToNode_ {uuid: $uuid})
            DETACH DELETE e
            """,
            uuid=self.uuid,
        )
    else:
        await driver.execute_query(
            """
            MATCH (n)-[e:MENTIONS|RELATES_TO|HAS_MEMBER {uuid: $uuid}]->(m)
            DELETE e
            """,
            uuid=self.uuid,
        )

    logger.debug(f'Deleted Edge: {self.uuid}')
