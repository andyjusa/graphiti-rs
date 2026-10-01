# Source excerpt: core/search/search_utils.py
# Graphiti core portions: Copyright 2024 Zep, Inc.; Apache-2.0.
# Runtime patches: user-owned operational source; synthetic tests only.

def rrf(
    results: list[list[str]], rank_const=1, min_score: float = 0
) -> tuple[list[str], list[float]]:
    scores: dict[str, float] = defaultdict(float)
    for result in results:
        for i, uuid in enumerate(result):
            scores[uuid] += 1 / (i + rank_const)

    scored_uuids = [term for term in scores.items()]
    scored_uuids.sort(reverse=True, key=lambda term: term[1])

    sorted_uuids = [term[0] for term in scored_uuids]

    return [uuid for uuid in sorted_uuids if scores[uuid] >= min_score], [
        scores[uuid] for uuid in sorted_uuids if scores[uuid] >= min_score
    ]
