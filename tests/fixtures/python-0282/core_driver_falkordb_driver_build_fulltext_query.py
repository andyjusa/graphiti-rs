def build_fulltext_query(
    self, query: str, group_ids: list[str] | None = None, max_query_length: int = 128
) -> str:
    """
    Build a fulltext query string for FalkorDB using RedisSearch syntax.
    FalkorDB uses RedisSearch-like syntax where:
    - Field queries use @ prefix: @field:value
    - Multiple values for same field: (@field:value1|value2)
    - Text search doesn't need @ prefix for content fields
    - AND is implicit with space: (@group_id:value) (text)
    - OR uses pipe within parentheses: (@group_id:value1|value2)
    """
    validate_group_ids(group_ids)

    if group_ids is None or len(group_ids) == 0:
        group_filter = ''
    else:
        # Escape group_ids with quotes to prevent RediSearch syntax errors
        # with reserved words like "main" or special characters like hyphens
        escaped_group_ids = [f'"{gid}"' for gid in group_ids]
        group_values = '|'.join(escaped_group_ids)
        group_filter = f'(@group_id:{group_values})'

    sanitized_query = self.sanitize(query)

    # Remove stopwords and empty tokens from the sanitized query
    query_words = sanitized_query.split()
    filtered_words = [word for word in query_words if word and word.lower() not in STOPWORDS]
    sanitized_query = ' | '.join(filtered_words)

    # If the query is too long return no query
    if len(sanitized_query.split(' ')) + len(group_ids or '') >= max_query_length:
        return ''

    full_query = group_filter + ' (' + sanitized_query + ')'

    return full_query
