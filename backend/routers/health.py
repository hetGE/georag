"""Health check endpoint for llama-server connectivity (chat + embeddings)."""
import asyncio
import httpx
from fastapi import APIRouter

from backend.config import (
    CHAT_BASE_URL,
    EMBEDDING_BASE_URL,
    EMBEDDING_URL,
    CHAT_MODEL,
    EMBEDDING_MODEL,
)

router = APIRouter(tags=["health"])

_health_client = httpx.AsyncClient(timeout=5.0)

_README_HINT = (
    "See the 'Step 1: Set Up llama-server' section of the README for the launch commands."
)


async def _probe_chat() -> str | None:
    """Returns None on success, or an error string explaining the failure."""
    try:
        response = await _health_client.get(f"{CHAT_BASE_URL}/v1/models")
        response.raise_for_status()
        data = response.json()
    except httpx.ConnectError:
        return f"Cannot connect to chat llama-server at {CHAT_BASE_URL}. Is it running?"
    except httpx.TimeoutException:
        return f"Chat llama-server at {CHAT_BASE_URL} timed out."
    except Exception as e:
        return f"Chat llama-server at {CHAT_BASE_URL} failed: {e}"

    model_ids = [m.get("id", "") for m in data.get("data", [])]
    if not model_ids:
        return f"Chat llama-server is running but no model is loaded. Expected alias '{CHAT_MODEL}'."
    if not any(CHAT_MODEL in mid for mid in model_ids):
        return (
            f"Chat model alias '{CHAT_MODEL}' not found on chat llama-server. "
            f"Loaded: {', '.join(model_ids)}."
        )
    return None


async def _probe_embedding() -> str | None:
    """Returns None on success, or an error string explaining the failure."""
    try:
        response = await _health_client.get(f"{EMBEDDING_BASE_URL}/v1/models")
        response.raise_for_status()
        data = response.json()
    except httpx.ConnectError:
        return f"Cannot connect to embedding llama-server at {EMBEDDING_BASE_URL}. Is it running?"
    except httpx.TimeoutException:
        return f"Embedding llama-server at {EMBEDDING_BASE_URL} timed out."
    except Exception as e:
        return f"Embedding llama-server at {EMBEDDING_BASE_URL} failed: {e}"

    model_ids = [m.get("id", "") for m in data.get("data", [])]
    if not model_ids:
        return (
            f"Embedding llama-server is running but no model is loaded. "
            f"Expected alias '{EMBEDDING_MODEL}'."
        )
    if not any(EMBEDDING_MODEL in mid for mid in model_ids):
        return (
            f"Embedding model alias '{EMBEDDING_MODEL}' not found on embedding llama-server. "
            f"Loaded: {', '.join(model_ids)}."
        )

    try:
        response = await _health_client.post(
            EMBEDDING_URL,
            json={"model": EMBEDDING_MODEL, "input": "test"},
        )
        response.raise_for_status()
    except Exception as e:
        return f"Embedding pipeline test call failed at {EMBEDDING_URL}: {e}"
    return None


@router.get("/health/llm")
async def check_llm_servers():
    """Verify both llama-server instances are reachable and serving the expected models."""
    chat_err, emb_err = await asyncio.gather(_probe_chat(), _probe_embedding())

    if chat_err is None and emb_err is None:
        return {"status": "ok"}

    parts = [e for e in (chat_err, emb_err) if e]
    detail = " ".join(parts) + " " + _README_HINT
    return {"status": "error", "detail": detail.strip()}
