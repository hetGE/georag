"""LM Studio chat + vision wrapper."""
import json
from typing import AsyncGenerator

import httpx

from backend.config import CHAT_URL, CHAT_MODEL

_client = httpx.AsyncClient(timeout=120.0)


async def stream_chat_response(messages: list[dict]) -> AsyncGenerator[str, None]:
    """Stream chat response from LM Studio. Yields tokens."""
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
                    yield content
            except json.JSONDecodeError:
                continue


async def chat_completion(messages: list[dict], max_tokens: int = 1024) -> str:
    """Non-streaming chat completion. Returns full response text."""
    response = await _client.post(CHAT_URL, json={
        "model": CHAT_MODEL,
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": max_tokens,
    })
    response.raise_for_status()
    data = response.json()
    return data["choices"][0]["message"]["content"]


async def vision_describe(image_b64: str, prompt: str = "Describe this engineering diagram or figure in detail.") -> str:
    """Use vision model to describe an image."""
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
