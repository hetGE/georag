"""Chat + vision wrapper for the chat model server (llama-server or mlx_lm.server)."""
import asyncio
import json
import logging
import time
from typing import AsyncGenerator

import httpx

from backend.config import CHAT_URL, CHAT_THINKING
from backend.services import llama_supervisor

logger = logging.getLogger(__name__)

# Read timeout is generous because Qwen3.5-9B can need 10+ minutes for a
# long generation (tagging, exploration). Connect/write/pool stay tight so we fail fast if
# the chat server is not actually up.
_client = httpx.AsyncClient(
    timeout=httpx.Timeout(connect=10.0, read=900.0, write=30.0, pool=10.0)
)


# mlx_lm.server does not fail a request whose generation thread has died (seen
# with a Metal "Resource limit exceeded" on very long replies): the connection
# just goes silent while /health keeps answering. While it works it is never
# quiet for long (keepalives during prompt processing, tokens after), so a
# silence this long means it is dead.
_MLX_STREAM_TIMEOUT = httpx.Timeout(connect=10.0, read=180.0, write=30.0, pool=10.0)


async def stream_chat_response(
    messages: list[dict],
    result: dict | None = None,
    *,
    max_tokens: int | None = None,
) -> AsyncGenerator[str, None]:
    """Stream chat response from the chat server. Yields tokens.

    max_tokens is None for no limit (the reply then ends when the model stops
    or the context window is full); the MLX backend is given one, see chat.py.

    The model answers directly unless config.CHAT_THINKING is on. When it is,
    both backends stream the reasoning separately from the answer (llama-server
    as delta.reasoning_content, mlx_lm.server as delta.reasoning); it is
    yielded wrapped in <think>...</think>, which the chat UI renders as a
    collapsible block.

    If `result` is given, result["finish_reason"] is set when the stream ends
    ("length" means the reply was cut off rather than finished).
    """
    model = llama_supervisor.active_chat_model()
    try:
        async for token in _stream_chat_tokens(model, messages, result, max_tokens):
            yield token
    except httpx.ReadTimeout:
        if model["backend"] != "mlx":
            raise
        logger.error("MLX chat server went silent mid-reply; restarting it")
        asyncio.create_task(llama_supervisor.restart_chat_server())
        raise RuntimeError(
            f"{model['label']} stopped responding part-way through the reply (the MLX "
            "runtime can fail on very long replies). The model is being restarted; ask "
            "again, and use a lower Retrieval Depth if it happens repeatedly.")


async def _stream_deltas(
    model: dict, payload: dict, result: dict,
) -> AsyncGenerator[tuple[str, str], None]:
    """POST one streaming chat request and yield ("reasoning" | "content", text)
    for each delta. Sets result["finish_reason"] when the server reports one."""
    timeout = _MLX_STREAM_TIMEOUT if model["backend"] == "mlx" else httpx.USE_CLIENT_DEFAULT
    async with _client.stream("POST", CHAT_URL, json=payload, timeout=timeout) as response:
        if response.is_error:
            # Surface the server's own explanation (e.g. the prompt exceeds
            # the context window) instead of a bare HTTP status.
            body = (await response.aread()).decode("utf-8", "replace")
            try:
                error = json.loads(body)["error"]
                detail = error["message"] if isinstance(error, dict) else str(error)
            except (ValueError, KeyError, TypeError):
                detail = body[:500]
            logger.error("LLM stream request failed: %s: %s", response.status_code, detail)
            raise RuntimeError(f"The chat LLM server rejected the request: {detail}")
        async for line in response.aiter_lines():
            if not line.startswith("data: "):
                continue
            data_str = line[6:]
            if data_str.strip() == "[DONE]":
                break
            try:
                data = json.loads(data_str)
            except json.JSONDecodeError:
                continue
            choice = (data.get("choices") or [{}])[0]
            if choice.get("finish_reason"):
                result["finish_reason"] = choice["finish_reason"]
            delta = choice.get("delta") or {}
            reasoning = delta.get("reasoning_content") or delta.get("reasoning")
            if reasoning:
                yield "reasoning", reasoning
            content = delta.get("content")
            if content:
                yield "content", content


async def _stream_chat_tokens(
    model: dict,
    messages: list[dict],
    result: dict | None,
    max_tokens: int | None,
) -> AsyncGenerator[str, None]:
    logger.info("LLM stream request: model=%s, messages=%d", model["label"], len(messages))
    t0 = time.time()
    thinking_count = 0
    token_count = 0
    in_think = False
    outcome: dict = {}
    payload: dict = {
        "model": model["request_model"],
        "messages": messages,
        "stream": True,
        "temperature": 0.3,
    }
    if max_tokens:
        payload["max_tokens"] = max_tokens
    if not CHAT_THINKING:
        # Qwen's chat template switch; both servers pass it to the template.
        payload["chat_template_kwargs"] = {"enable_thinking": False}

    async for kind, text in _stream_deltas(model, payload, outcome):
        if kind == "reasoning":
            thinking_count += 1
            if not in_think:
                in_think = True
                text = "<think>" + text
        else:
            token_count += 1
            if in_think:
                in_think = False
                text = "</think>\n\n" + text
        yield text

    if in_think:
        # Stream ended while still reasoning (e.g. the context filled up mid-thought).
        yield "</think>\n\n"
    if result is not None:
        result["finish_reason"] = outcome.get("finish_reason")
    logger.info("LLM stream complete: ~%d thinking + ~%d answer tokens in %.1fs (finish_reason=%s)",
                thinking_count, token_count, time.time() - t0, outcome.get("finish_reason"))


async def chat_completion(
    messages: list[dict],
    max_tokens: int = 1024,
    *,
    chat_template_kwargs: dict | None = None,
) -> str:
    """Non-streaming chat completion. Returns full response text.

    chat_template_kwargs is forwarded in the request body (e.g.
    {"enable_thinking": False} to disable Qwen3 reasoning per-request without
    changing the server's launch flags).
    """
    model = llama_supervisor.active_chat_model()
    logger.info("LLM completion request: model=%s, max_tokens=%d", model["label"], max_tokens)
    t0 = time.time()
    payload: dict = {
        "model": model["request_model"],
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": max_tokens,
    }
    if chat_template_kwargs:
        payload["chat_template_kwargs"] = chat_template_kwargs
    response = await _client.post(CHAT_URL, json=payload)
    if response.is_error:
        logger.error("LLM request failed: %s — %s", response.status_code, response.text)
    response.raise_for_status()
    data = response.json()
    result = data["choices"][0]["message"]["content"]
    logger.info("LLM completion done: %d chars in %.1fs", len(result), time.time() - t0)
    return result


async def vision_describe(image_b64: str, prompt: str = "Describe this engineering diagram or figure in detail.") -> str:
    """Use vision model to describe an image."""
    model = llama_supervisor.active_chat_model()
    if not model["vision"]:
        raise RuntimeError(
            f"{model['label']} is served text-only; select a chat model that "
            "describes images to index image content")
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
