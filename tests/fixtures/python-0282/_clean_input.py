def _clean_input(self, input: str) -> str:
    """Clean input string of invalid unicode and control characters.

    Args:
        input: Raw input string to be cleaned

    Returns:
        Cleaned string safe for LLM processing
    """
    # Clean any invalid Unicode
    cleaned = input.encode('utf-8', errors='ignore').decode('utf-8')

    # Remove zero-width characters and other invisible unicode
    zero_width = '\u200b\u200c\u200d\ufeff\u2060'
    for char in zero_width:
        cleaned = cleaned.replace(char, '')

    # Remove control characters except newlines, returns, and tabs
    cleaned = ''.join(char for char in cleaned if ord(char) >= 32 or char in '\n\r\t')

    return cleaned
