"""Health check endpoint for LM Studio connectivity."""
import httpx
from fastapi import APIRouter

from backend.config import LM_STUDIO_BASE_URL, EMBEDDING_URL, CHAT_URL, EMBEDDING_MODEL, CHAT_MODEL

router = APIRouter(tags=["health"])

_health_client = httpx.AsyncClient(timeout=5.0)


@router.get("/health/lm-studio")
async def check_lm_studio():
    """Verify LM Studio is running and both models are loaded by making lightweight test calls."""
    # 1. Check connectivity and get loaded models list
    try:
        response = await _health_client.get(f"{LM_STUDIO_BASE_URL}/v1/models")
        response.raise_for_status()
        data = response.json()
        model_ids = [m.get("id", "") for m in data.get("data", [])]
    except httpx.ConnectError:
        return {"status": "error", "detail": "Cannot connect to LM Studio. Is it running?"}
    except httpx.TimeoutException:
        return {"status": "error", "detail": "LM Studio connection timed out."}
    except Exception as e:
        return {"status": "error", "detail": f"LM Studio connection failed: {e}"}

    if not model_ids:
        return {"status": "error", "detail": "LM Studio is running but no models are loaded."}

    # 2. Test embedding model with a lightweight call (fast, sub-second)
    try:
        response = await _health_client.post(EMBEDDING_URL, json={
            "model": EMBEDDING_MODEL,
            "input": "test",
        })
        response.raise_for_status()
    except Exception:
        return {
            "status": "error",
            "detail": f"Embedding model '{EMBEDDING_MODEL}' is not loaded. Please load it in LM Studio.",
        }

    # 3. Verify chat model appears in loaded models list
    # (actual completion test is too slow due to cold-start prompt processing)
    # LM Studio may prefix with org, e.g. "qwen/qwen3-vl-30b" for config "qwen3-vl-30b"
    chat_found = any(CHAT_MODEL in mid for mid in model_ids)
    if not chat_found:
        return {
            "status": "error",
            "detail": f"Chat model '{CHAT_MODEL}' is not loaded. Please load it in LM Studio.",
        }

    return {"status": "ok"}
