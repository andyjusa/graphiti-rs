# Observed user-owned runtime patch: patches/openrouter_json.py
def flatten_schema_value_envelope(parsed: Any, schema: dict[str, Any]) -> Any:
    """Flatten only a schema-shaped Qwen envelope for the expected output fields."""
    if not isinstance(parsed, dict) or not isinstance(parsed.get("properties"), dict):
        return parsed
    expected_properties = schema.get("properties")
    if not isinstance(expected_properties, dict) or not expected_properties:
        return parsed
    if "properties" in expected_properties:
        return parsed
    allowed_envelope_keys = {
        "$defs",
        "additionalProperties",
        "definitions",
        "description",
        "properties",
        "required",
        "title",
        "type",
    }
    if parsed.get("type") != "object" or not set(parsed).issubset(
        allowed_envelope_keys
    ):
        return parsed
    properties = parsed["properties"]
    if set(properties) != set(expected_properties):
        return parsed
    if not properties or not all(
        isinstance(item, dict) and "value" in item for item in properties.values()
    ):
        return parsed
    return {name: item["value"] for name, item in properties.items()}
