# Source excerpt: patches/group_read_routing.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

def normalize_group_ids(
    group_ids: list[str] | None,
    default_group_id: str | None,
    validator: Callable[[str], Any],
) -> list[str]:
    requested = (
        group_ids
        if group_ids is not None
        else [default_group_id]
        if default_group_id
        else []
    )
    normalized: list[str] = []
    seen: set[str] = set()
    for group_id in requested:
        if not isinstance(group_id, str) or not group_id:
            raise GroupScopeViolation("group_id_must_be_non_empty_string")
        try:
            validator(group_id)
        except Exception as error:
            raise GroupScopeViolation("group_id_invalid") from error
        if group_id in seen:
            continue
        seen.add(group_id)
        normalized.append(group_id)
    return normalized
