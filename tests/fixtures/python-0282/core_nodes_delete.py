# Source excerpt: core/nodes.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

async def delete(self, driver: GraphDriver):
    if driver.graph_operations_interface:
        try:
            return await driver.graph_operations_interface.node_delete(self, driver)
        except NotImplementedError:
            pass

    match driver.provider:
        case GraphProvider.NEO4J:
            records, _, _ = await driver.execute_query(
                """
                MATCH (n {uuid: $uuid})
                WHERE n:Entity OR n:Episodic OR n:Community
                OPTIONAL MATCH (n)-[r]-()
                WITH collect(r.uuid) AS edge_uuids, n
                DETACH DELETE n
                RETURN edge_uuids
                """,
                uuid=self.uuid,
            )

        case GraphProvider.KUZU:
            for label in ['Episodic', 'Community']:
                await driver.execute_query(
                    f"""
                    MATCH (n:{label} {{uuid: $uuid}})
                    DETACH DELETE n
                    """,
                    uuid=self.uuid,
                )
            # Entity edges are actually nodes in Kuzu, so simple `DETACH DELETE` will not work.
            # Explicitly delete the "edge" nodes first, then the entity node.
            await driver.execute_query(
                """
                MATCH (n:Entity {uuid: $uuid})-[:RELATES_TO]->(e:RelatesToNode_)
                DETACH DELETE e
                """,
                uuid=self.uuid,
            )
            await driver.execute_query(
                """
                MATCH (n:Entity {uuid: $uuid})
                DETACH DELETE n
                """,
                uuid=self.uuid,
            )
        case _:  # FalkorDB, Neptune
            for label in ['Entity', 'Episodic', 'Community']:
                await driver.execute_query(
                    f"""
                    MATCH (n:{label} {{uuid: $uuid}})
                    DETACH DELETE n
                    """,
                    uuid=self.uuid,
                )

    logger.debug(f'Deleted Node: {self.uuid}')
