"""LM Studio embedding wrapper with concurrent batching."""
import asyncio

import httpx

from backend.config import EMBEDDING_URL, EMBEDDING_MODEL, EMBEDDING_BATCH_SIZE

_client = httpx.AsyncClient(timeout=300.0)
_embed_semaphore = asyncio.Semaphore(3)


async def embed_text(text: str) -> list[float]:
    """Embed a single text string. Returns embedding vector."""
    response = await _client.post(EMBEDDING_URL, json={
        "model": EMBEDDING_MODEL,
        "input": text,
    })
    response.raise_for_status()
    data = response.json()
    return data["data"][0]["embedding"]


async def _embed_one_batch(batch: list[str]) -> list[list[float]]:
    """Embed a single batch, respecting the concurrency semaphore."""
    async with _embed_semaphore:
        response = await _client.post(EMBEDDING_URL, json={
            "model": EMBEDDING_MODEL,
            "input": batch,
        })
        response.raise_for_status()
        data = response.json()
        return [item["embedding"] for item in sorted(data["data"], key=lambda x: x["index"])]


async def embed_batch(texts: list[str]) -> list[list[float]]:
    """Embed a batch of texts with concurrent sub-batches. Returns list of embedding vectors."""
    if not texts:
        return []

    # Split into sub-batches
    batches = [texts[i:i + EMBEDDING_BATCH_SIZE] for i in range(0, len(texts), EMBEDDING_BATCH_SIZE)]

    # Run sub-batches concurrently (semaphore limits to 3 at a time)
    results = await asyncio.gather(*[_embed_one_batch(b) for b in batches])

    # Flatten
    all_embeddings = []
    for batch_result in results:
        all_embeddings.extend(batch_result)

    return all_embeddings
