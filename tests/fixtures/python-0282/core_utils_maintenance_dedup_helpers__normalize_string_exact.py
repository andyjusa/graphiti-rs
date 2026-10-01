# Source excerpt: core/utils/maintenance/dedup_helpers.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

def _normalize_string_exact(name: str) -> str:
    """Lowercase text and collapse whitespace so equal names map to the same key."""
    normalized = re.sub(r'[\s]+', ' ', name.lower())
    return normalized.strip()
