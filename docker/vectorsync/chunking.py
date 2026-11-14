"""Split long text into embedding-sized chunks (no tokenizer download, no dependencies).

Recursive on natural boundaries: paragraphs, then lines, then sentences, then words; pieces are
packed greedily up to `max_chars`, and each chunk after the first starts with up to `overlap`
characters from the end of the previous one (cut at a word boundary) so a sentence split across
two chunks is still findable from either.
"""

from __future__ import annotations

SEPARATORS = ("\n\n", "\n", ". ", "; ", ", ", " ")
DEFAULT_MAX_CHARS = 1500
DEFAULT_OVERLAP = 150


def _split_hard(text: str, max_chars: int) -> list[str]:
    return [text[i : i + max_chars] for i in range(0, len(text), max_chars)]


def _pieces(text: str, max_chars: int, seps: tuple[str, ...]) -> list[str]:
    if len(text) <= max_chars:
        return [text]
    if not seps:
        return _split_hard(text, max_chars)
    sep, rest = seps[0], seps[1:]
    parts = text.split(sep)
    if len(parts) == 1:
        return _pieces(text, max_chars, rest)
    out: list[str] = []
    for i, part in enumerate(parts):
        piece = part + (sep if i < len(parts) - 1 else "")
        if len(piece) > max_chars:
            out.extend(_pieces(piece, max_chars, rest))
        elif piece:
            out.append(piece)
    return out


def _tail(text: str, n: int) -> str:
    if n <= 0 or not text:
        return ""
    tail = text[-n:]
    cut = tail.find(" ")
    return tail[cut + 1 :] if 0 <= cut < len(tail) - 1 else tail


def split_text(text: str, max_chars: int = DEFAULT_MAX_CHARS, overlap: int = DEFAULT_OVERLAP) -> list[str]:
    text = (text or "").strip()
    if not text:
        return []
    max_chars = max(100, int(max_chars))
    overlap = max(0, min(int(overlap), max_chars // 2))
    if len(text) <= max_chars:
        return [text]
    budget = max_chars - overlap
    chunks: list[str] = []
    cur = ""
    for piece in _pieces(text, budget, SEPARATORS):
        if cur and len(cur) + len(piece) > budget:
            chunks.append(cur.strip())
            cur = ""
        cur += piece
    if cur.strip():
        chunks.append(cur.strip())
    if overlap:
        chunks = [chunks[0]] + [(_tail(chunks[i - 1], overlap) + " " + chunks[i]).strip() for i in range(1, len(chunks))]
    return [c for c in chunks if c]


def merge_small(pieces: list[str], max_chars: int, sep: str = "\n\n") -> list[str]:
    """Join consecutive short pieces (e.g. one Docling paragraph each) up to max_chars."""
    out: list[str] = []
    cur = ""
    for p in pieces:
        p = p.strip()
        if not p:
            continue
        if cur and len(cur) + len(sep) + len(p) > max_chars:
            out.append(cur)
            cur = p
        else:
            cur = f"{cur}{sep}{p}" if cur else p
    if cur:
        out.append(cur)
    return out
