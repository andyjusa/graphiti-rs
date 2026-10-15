# Observed user-owned adapter decoder block.
cleaned = content.strip()
if cleaned.startswith("```"):
    cleaned = (
        cleaned.split("\n", 1)[1] if "\n" in cleaned else cleaned
    )
    if cleaned.endswith("```"):
        cleaned = cleaned[:-3].rstrip()
starts = [
    position
    for position in (cleaned.find("{"), cleaned.find("["))
    if position >= 0
]
if starts:
    cleaned = cleaned[min(starts) :]
end = max(cleaned.rfind("}"), cleaned.rfind("]"))
if end >= 0:
    cleaned = cleaned[: end + 1]
parsed = decode_first_json(cleaned)
return normalize_structured_response(parsed, schema)
