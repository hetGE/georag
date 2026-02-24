"""LM Studio embedding wrapper."""
import httpx

from backend.config import EMBEDDING_URL, EMBEDDING_MODEL, EMBEDDING_BATCH_SIZE

_client = httpx.AsyncClient(timeout=60.0)


async def embed_text(text: str) -> list[float]:
    """Embed a single text string. Returns embedding vector."""
    response = await _client.post(EMBEDDING_URL, json={
        "model": EMBEDDING_MODEL,
        "input": text,
    })
    response.raise_for_status()
    data = response.json()
    return data["data"][0]["embedding"]


async def embed_batch(texts: list[str]) -> list[list[float]]:
    """Embed a batch of texts. Returns list of embedding vectors."""
    all_embeddings = []

    for i in range(0, len(texts), EMBEDDING_BATCH_SIZE):
        batch = texts[i:i + EMBEDDING_BATCH_SIZE]
        response = await _client.post(EMBEDDING_URL, json={
            "model": EMBEDDING_MODEL,
            "input": batch,
        })
        response.raise_for_status()
        data = response.json()
        batch_embeddings = [item["embedding"] for item in sorted(data["data"], key=lambda x: x["index"])]
        all_embeddings.extend(batch_embeddings)

    return all_embeddings
