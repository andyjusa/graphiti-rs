# Source excerpt: patches/group_read_routing.py
# Graphiti core portions: Copyright 2024, Zep Software, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

def require_exactly_one_group(
    group_ids: list[str] | None,
    validator: Callable[[str], Any],
) -> str:
    """Return one explicitly supplied group or fail closed.

    Destructive operations must never inherit a mutable process default.  The
    raw list is checked before normalization so duplicate values do not turn a
    multi-value request into an apparently safe single group.
    """

    if not isinstance(group_ids, list) or len(group_ids) != 1:
        raise GroupScopeViolation("exactly_one_explicit_group_id_required")
    normalized = normalize_group_ids(group_ids, None, validator)
    if len(normalized) != 1:
        raise GroupScopeViolation("exactly_one_explicit_group_id_required")
    return normalized[0]
