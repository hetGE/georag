"""llama-server chat + vision wrapper."""
import json
import logging
import time
from typing import AsyncGenerator

import httpx

from backend.config import CHAT_URL, CHAT_MODEL

logger = logging.getLogger(__name__)

# Read timeout is generous because Qwen3.5-9B with thinking can need several
# minutes for a 4-8K-token wiki page. Connect/write/pool stay tight so we fail
# fast if the chat llama-server is not actually up.
_client = httpx.AsyncClient(
    timeout=httpx.Timeout(connect=10.0, read=600.0, write=30.0, pool=10.0)
)


async def stream_chat_response(messages: list[dict]) -> AsyncGenerator[str, None]:
    """Stream chat response from llama-server. Yields tokens."""
    logger.info("LLM stream request: model=%s, messages=%d", CHAT_MODEL, len(messages))
    t0 = time.time()
    token_count = 0
    async with _client.stream(
        "POST",
        CHAT_URL,
        json={
            "model": CHAT_MODEL,
            "messages": messages,
            "stream": True,
            "temperature": 0.3,
            "max_tokens": 4096,
        },
    ) as response:
        response.raise_for_status()
        async for line in response.aiter_lines():
            if not line.startswith("data: "):
                continue
            data_str = line[6:]
            if data_str.strip() == "[DONE]":
                break
            try:
                data = json.loads(data_str)
                delta = data.get("choices", [{}])[0].get("delta", {})
                content = delta.get("content", "")
                if content:
                    token_count += 1
                    yield content
            except json.JSONDecodeError:
                continue
    logger.info("LLM stream complete: ~%d tokens in %.1fs", token_count, time.time() - t0)


async def chat_completion(messages: list[dict], max_tokens: int = 1024) -> str:
    """Non-streaming chat completion. Returns full response text."""
    logger.info("LLM completion request: model=%s, max_tokens=%d", CHAT_MODEL, max_tokens)
    t0 = time.time()
    response = await _client.post(CHAT_URL, json={
        "model": CHAT_MODEL,
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": max_tokens,
    })
    if response.is_error:
        logger.error("LLM request failed: %s — %s", response.status_code, response.text)
    response.raise_for_status()
    data = response.json()
    result = data["choices"][0]["message"]["content"]
    logger.info("LLM completion done: %d chars in %.1fs", len(result), time.time() - t0)
    return result


async def vision_describe(image_b64: str, prompt: str = "Describe this engineering diagram or figure in detail.") -> str:
    """Use vision model to describe an image."""
    logger.info("Vision describe request (image size: %d bytes b64)", len(image_b64))
    messages = [
        {
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{image_b64}"}},
            ],
        }
    ]
    return await chat_completion(messages, max_tokens=512)
