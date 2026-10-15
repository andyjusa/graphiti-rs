# Observed user-owned runtime patch: patches/openrouter_json.py
def decode_first_json(text: str) -> Any:
    """Decode the first complete JSON value, ignoring duplicated trailing output."""
    value, _ = __import__("json").JSONDecoder().raw_decode(text)
    return value
