# Observed user-owned runtime patch: patches/openrouter_json.py
def normalize_structured_response(parsed: Any, schema: dict[str, Any]) -> dict[str, Any]:
    """Return a schema-shaped object for known semantically equivalent outputs."""
    parsed = flatten_schema_value_envelope(parsed, schema)
    properties = schema.get("properties", {})
    if isinstance(parsed, list) and isinstance(properties, dict) and len(properties) == 1:
        return {next(iter(properties)): parsed}

    if not isinstance(parsed, dict):
        raise ValueError("OpenRouter structured output was not an object")

    if schema.get("title") == "SummarizedEntities" and "summaries" not in parsed:
        if all(isinstance(name, str) and isinstance(summary, str) for name, summary in parsed.items()):
            return {
                "summaries": [
                    {"name": name, "summary": summary} for name, summary in parsed.items()
                ]
            }

    return parsed
