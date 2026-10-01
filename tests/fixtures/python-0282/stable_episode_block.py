# Copyright 2024 Zep, Inc.; Apache-2.0. Operational stable UUID patch.
if uuid is not None:
    try:
        episode = await EpisodicNode.get_by_uuid(self.driver, uuid)
    except NodeNotFoundError:
        episode = EpisodicNode(
            uuid=uuid,
            name=name,
            group_id=group_id,
            labels=[],
            source=source,
            content=episode_body,
            source_description=source_description,
            created_at=now,
            valid_at=reference_time,
        )
else:
    episode = EpisodicNode(
        name=name,
        group_id=group_id,
        labels=[],
        source=source,
        content=episode_body,
        source_description=source_description,
        created_at=now,
        valid_at=reference_time,
    )
