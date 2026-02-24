"""Text chunking using RecursiveCharacterTextSplitter."""
import re

from langchain_text_splitters import RecursiveCharacterTextSplitter

from backend.config import CHUNK_SIZE, CHUNK_OVERLAP

_splitter = RecursiveCharacterTextSplitter(
    chunk_size=CHUNK_SIZE,
    chunk_overlap=CHUNK_OVERLAP,
    length_function=len,
    separators=["\n\n", "\n", ". ", " ", ""],
)

# Patterns embedded by extractors: [Page N], [Slide N], [Sheet: name]
_PAGE_RE = re.compile(r"\[Page\s+(\d+)\]", re.IGNORECASE)
_SLIDE_RE = re.compile(r"\[Slide\s+(\d+)\]", re.IGNORECASE)
_SHEET_RE = re.compile(r"\[Sheet:\s*(.+?)\]", re.IGNORECASE)


def _extract_page(chunk_text: str) -> str:
    """Return the last page/slide/sheet marker found in the chunk text, or ''."""
    m = None
    for m in _PAGE_RE.finditer(chunk_text):
        pass
    if m:
        return m.group(1)
    for m in _SLIDE_RE.finditer(chunk_text):
        pass
    if m:
        return f"slide {m.group(1)}"
    for m in _SHEET_RE.finditer(chunk_text):
        pass
    if m:
        return m.group(1)
    return ""


def chunk_text(text: str, metadata: dict = None) -> list[dict]:
    """Split text into chunks with metadata.

    Returns list of dicts with 'text' and 'metadata' keys.
    """
    if not text or not text.strip():
        return []

    chunks = _splitter.split_text(text)
    result = []
    base_metadata = metadata or {}

    for i, chunk in enumerate(chunks):
        chunk_meta = {**base_metadata, "chunk_index": i}
        page = _extract_page(chunk)
        if page:
            chunk_meta["page"] = page
        result.append({"text": chunk, "metadata": chunk_meta})

    return result
