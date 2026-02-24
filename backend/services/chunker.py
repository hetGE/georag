"""Text chunking using RecursiveCharacterTextSplitter."""
from langchain_text_splitters import RecursiveCharacterTextSplitter

from backend.config import CHUNK_SIZE, CHUNK_OVERLAP

_splitter = RecursiveCharacterTextSplitter(
    chunk_size=CHUNK_SIZE,
    chunk_overlap=CHUNK_OVERLAP,
    length_function=len,
    separators=["\n\n", "\n", ". ", " ", ""],
)


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
        result.append({"text": chunk, "metadata": chunk_meta})

    return result
