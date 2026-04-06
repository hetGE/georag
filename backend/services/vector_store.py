"""ChromaDB per-tag collections."""
import logging

import chromadb
from chromadb.config import Settings

from backend.config import CHROMA_DIR, EMBEDDING_DIM, TOP_K_PER_TAG

logger = logging.getLogger(__name__)

_client = chromadb.PersistentClient(
    path=str(CHROMA_DIR),
    settings=Settings(anonymized_telemetry=False),
)


def _collection_name(tag_name: str) -> str:
    """Convert tag name to a valid ChromaDB collection name."""
    return f"tag_{tag_name}"


def get_or_create_collection(tag_name: str):
    """Get or create a ChromaDB collection for a tag."""
    return _client.get_or_create_collection(
        name=_collection_name(tag_name),
        metadata={"hnsw:space": "cosine"},
    )


def add_chunks(tag_name: str, ids: list[str], embeddings: list[list[float]],
               documents: list[str], metadatas: list[dict]):
    """Add chunks to a tag's collection using upsert for idempotent reprocessing."""
    collection = get_or_create_collection(tag_name)
    batch_size = 5000
    for i in range(0, len(ids), batch_size):
        collection.upsert(
            ids=ids[i:i + batch_size],
            embeddings=embeddings[i:i + batch_size],
            documents=documents[i:i + batch_size],
            metadatas=metadatas[i:i + batch_size],
        )


def query_tags(query_embedding: list[float], tag_names: list[str], top_k: int | None = None) -> list[dict]:
    """Query multiple tag collections, merge and deduplicate results."""
    logger.info("Vector query: tags=%s, top_k=%s", tag_names, top_k or TOP_K_PER_TAG)
    all_results = []

    for tag_name in tag_names:
        try:
            collection = _client.get_collection(_collection_name(tag_name))
        except Exception:
            continue

        results = collection.query(
            query_embeddings=[query_embedding],
            n_results=top_k or TOP_K_PER_TAG,
            include=["documents", "metadatas", "distances"],
        )

        if not results["ids"][0]:
            continue

        for idx in range(len(results["ids"][0])):
            metadata = results["metadatas"][0][idx] if results["metadatas"] else {}
            all_results.append({
                "text": results["documents"][0][idx],
                "file_path": metadata.get("file_path", ""),
                "filename": metadata.get("filename", ""),
                "page": metadata.get("page", ""),
                "chunk_index": metadata.get("chunk_index", 0),
                "score": 1 - results["distances"][0][idx],  # Convert distance to similarity
                "tag": tag_name,
            })

    # Deduplicate by (file_path, chunk_index)
    seen = set()
    deduped = []
    for r in sorted(all_results, key=lambda x: x["score"], reverse=True):
        key = (r["file_path"], r["chunk_index"])
        if key not in seen:
            seen.add(key)
            deduped.append(r)

    logger.info("Vector query returned %d results (%d before dedup)", len(deduped), len(all_results))
    return deduped


def search_file_paths(query_embedding: list[float], tag_names: list[str], top_k_per_tag: int = 20) -> dict[str, float]:
    """Search tag collections and return unique file_paths with their best similarity score."""
    chunks = query_tags(query_embedding, tag_names, top_k=top_k_per_tag)
    file_scores: dict[str, float] = {}
    for c in chunks:
        fp = c["file_path"]
        if fp not in file_scores or c["score"] > file_scores[fp]:
            file_scores[fp] = c["score"]
    return file_scores


def delete_collection(tag_name: str):
    """Delete an entire tag collection from ChromaDB."""
    try:
        _client.delete_collection(_collection_name(tag_name))
    except Exception:
        pass


def delete_file_from_tag(tag_name: str, file_path: str):
    """Remove all chunks for a file from a tag's collection."""
    try:
        collection = _client.get_collection(_collection_name(tag_name))
        collection.delete(where={"file_path": file_path})
    except Exception:
        pass


def get_collection_count(tag_name: str) -> int:
    """Get the number of chunks in a tag's collection."""
    try:
        collection = _client.get_collection(_collection_name(tag_name))
        return collection.count()
    except Exception:
        return 0


def get_chunks_for_file(tag_name: str, file_path: str) -> list[dict]:
    """Retrieve all stored chunks for a file from a tag's collection, sorted by chunk_index."""
    try:
        collection = _client.get_collection(_collection_name(tag_name))
        results = collection.get(
            where={"file_path": file_path},
            include=["documents", "metadatas"],
        )
    except Exception:
        return []

    chunks = []
    for idx in range(len(results["ids"])):
        meta = results["metadatas"][idx] if results["metadatas"] else {}
        chunks.append({
            "text": results["documents"][idx],
            "page": meta.get("page", ""),
            "chunk_index": meta.get("chunk_index", 0),
            "file_path": meta.get("file_path", file_path),
            "filename": meta.get("filename", ""),
        })
    chunks.sort(key=lambda c: c["chunk_index"])
    return chunks


def get_file_chunks_any_tag(file_path: str, tag_names: list[str]) -> list[dict]:
    """Try each tag collection until chunks are found for the given file_path."""
    for tag_name in tag_names:
        chunks = get_chunks_for_file(tag_name, file_path)
        if chunks:
            return chunks
    return []
