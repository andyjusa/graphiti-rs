# Observed user-owned runtime patch; read shapes only, no installer executed.
_EDGE = re.compile(
    r"MATCH\s*\(n:Entity\)-\[e:RELATES_TO(?P<properties>\s*\{[^}]+\})?\]->\(m:Entity\)\s*(?P<where>WHERE\b)?",
    re.IGNORECASE,
)

_WRITES = re.compile(r"\b(?:CREATE|MERGE|SET|DELETE|REMOVE|DROP)\b", re.IGNORECASE)

_FULLTEXT_PREFIX = re.compile(
    r"\s*CALL\s+db\.idx\.fulltext\.queryRelationships\([^;]+\)\s*"
    r"YIELD\s+relationship\s+AS\s+rel,\s*score\s*",
    re.IGNORECASE,
)

def safe_edge_read(query: str) -> str:
    if _WRITES.search(query):
        return query
    matches = list(_EDGE.finditer(query))
    if len(matches) != 1:
        return query
    match = matches[0]
    prefix = query[: match.start()]
    if prefix.strip() and not _FULLTEXT_PREFIX.fullmatch(prefix):
        return query
    properties = match["properties"] or ""
    replacement = (
        f"MATCH ()-[e:RELATES_TO{properties}]->() "
        "WITH *, startNode(e) AS n, endNode(e) AS m "
        "WHERE 'Entity' IN labels(n) AND 'Entity' IN labels(m) "
        + ("AND " if match["where"] else "")
    )
    return prefix + replacement + query[match.end() :]
