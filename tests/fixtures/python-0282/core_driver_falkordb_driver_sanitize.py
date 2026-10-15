def sanitize(self, query: str) -> str:
    """
    Replace FalkorDB special characters with whitespace.
    Based on FalkorDB tokenization rules: ,.<>{}[]"':;!@#$%^&*()-+=~
    """
    # FalkorDB separator characters that break text into tokens
    separator_map = str.maketrans(
        {
            ',': ' ',
            '.': ' ',
            '<': ' ',
            '>': ' ',
            '{': ' ',
            '}': ' ',
            '[': ' ',
            ']': ' ',
            '"': ' ',
            "'": ' ',
            ':': ' ',
            ';': ' ',
            '!': ' ',
            '@': ' ',
            '#': ' ',
            '$': ' ',
            '%': ' ',
            '^': ' ',
            '&': ' ',
            '*': ' ',
            '(': ' ',
            ')': ' ',
            '-': ' ',
            '+': ' ',
            '=': ' ',
            '~': ' ',
            '?': ' ',
            '|': ' ',
            '/': ' ',
            '\\': ' ',
        }
    )
    sanitized = query.translate(separator_map)
    # Clean up multiple spaces
    sanitized = ' '.join(sanitized.split())
    return sanitized
