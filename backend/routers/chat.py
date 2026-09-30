"""Chat endpoint with SSE streaming."""
import asyncio
import json
import logging
import re
import time
from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session
from sse_starlette.sse import EventSourceResponse

logger = logging.getLogger(__name__)

from backend.models.database import get_db
from backend.models.schemas import Conversation, Message
from backend.models.pydantic_models import ChatRequest
from backend.services.llm_client import stream_chat_response
from backend.services.vector_store import query_tags
from backend.services.embedding_client import embed_text
from backend.services import llama_supervisor
from backend.config import CONVERSATION_HISTORY_TURNS, chat_depth_level

router = APIRouter(tags=["chat"])

_chat_streaming = False
_chat_conversation_id = None
_chat_listeners: list[asyncio.Queue] = []
_chat_cancel = False


_THINK_BLOCK_RE = re.compile(r"<think>.*?(?:</think>|\Z)", re.DOTALL)


def _strip_thinking(text: str) -> str:
    """Drop <think>...</think> reasoning from a stored assistant reply. It is
    kept in the DB for display, but must not be re-sent to the LLM as history
    (it would eat context and the model would mistake it for answer text)."""
    return _THINK_BLOCK_RE.sub("", text).strip()


def _estimate_tokens(text: str) -> int:
    """Deliberately high estimate of a text's token count. English prose runs
    ~4 characters per token, but tables, numbers and LaTeX tokenise far worse."""
    return len(text) // 3 + 8


def depth_refusal(level: dict, reason: str) -> str:
    """User-facing message for a Retrieval Depth level the machine cannot run."""
    return (f"The \"{level['label']}\" retrieval depth is not available on this machine "
            f"(it assumes {level['memory_text']}). {reason} "
            "Choose a lower Retrieval Depth.")


def build_context_prompt(chunks: list[dict]) -> str:
    if not chunks:
        return ""
    context_parts = []
    for i, chunk in enumerate(chunks, 1):
        source = chunk.get("filename", chunk.get("file_path", "unknown"))
        page = chunk.get("page", "")
        page_str = f" (page {page})" if page else ""
        context_parts.append(f"[Source {i}: {source}{page_str}]\n{chunk['text']}")
    return "\n\n---\n\n".join(context_parts)


@router.get("/chat/streaming")
async def chat_streaming_status():
    return {"streaming": _chat_streaming, "conversation_id": _chat_conversation_id}


@router.post("/chat/stop")
async def chat_stop():
    global _chat_cancel
    if not _chat_streaming:
        return {"ok": False, "reason": "not_streaming"}
    _chat_cancel = True
    return {"ok": True}


@router.post("/chat")
async def chat(request: ChatRequest, db: Session = Depends(get_db)):
    global _chat_streaming, _chat_conversation_id, _chat_cancel
    if _chat_streaming:
        return EventSourceResponse(
            iter([{"event": "error", "data": json.dumps({"error": "Another chat response is already in progress."})}])
        )
    # llama-servers may be paused (manual auto-shutdown or end of downtime
    # before resume). Start them on demand; the status pill will reflect
    # "Starting LLMs" while we wait.
    ready = await llama_supervisor.ensure_running()
    if not ready:
        return EventSourceResponse(
            iter([{"event": "error", "data": json.dumps({"error": "Local LLM servers did not start in time. Try again in a moment."})}])
        )

    # The depth level decides how much is retrieved and how long the reply may
    # be. Its context window must be loaded (and fit in memory) before the chat
    # starts; a level this machine cannot run is refused here.
    level = chat_depth_level(llama_supervisor.active_chat_model_key(), request.depth)
    from backend.routers.system import _busy_reason  # lazy: system imports this module
    ctx_ok, ctx_error = await llama_supervisor.ensure_chat_ctx(level["ctx"], _busy_reason)
    if not ctx_ok:
        logger.warning("Chat: depth '%s' refused: %s", level["key"], ctx_error)
        return EventSourceResponse(
            iter([{"event": "error", "data": json.dumps({"error": depth_refusal(level, ctx_error)})}])
        )
    _chat_streaming = True
    _chat_cancel = False

    # Get or create conversation
    if request.conversation_id:
        conversation = db.get(Conversation, request.conversation_id)
        if not conversation:
            conversation = Conversation(selected_tags=request.tag_names)
            db.add(conversation)
            db.commit()
            db.refresh(conversation)
    else:
        conversation = Conversation(selected_tags=request.tag_names)
        db.add(conversation)
        db.commit()
        db.refresh(conversation)

    # Save user message
    user_msg = Message(conversation_id=conversation.id, role="user", content=request.message)
    db.add(user_msg)
    db.commit()

    # RAG retrieval over the selected tags
    chunks = []
    sources = []

    if request.tag_names:
        try:
            logger.info("Chat: embedding query (%d chars)", len(request.message))
            t0 = time.time()
            query_embedding = await embed_text(request.message)
            logger.info("Chat: query embedded in %.1fs", time.time() - t0)

            t0 = time.time()
            chunks = query_tags(query_embedding, request.tag_names,
                                top_k=request.top_k_per_tag or level["top_k"])
            chunks = chunks[:request.max_context_chunks or level["max_chunks"]]
            sources = [
                {"file_path": c.get("file_path", ""), "filename": c.get("filename", ""),
                 "page": c.get("page", ""), "score": c.get("score", 0)}
                for c in chunks
            ]
            logger.info("Chat RAG: retrieved %d chunks in %.1fs", len(chunks), time.time() - t0)
        except Exception as e:
            logger.warning("Chat RAG: retrieval failed: %s", e)

    # Build messages for LLM
    system_prompt = (
        "You are GeoRAG, a geotechnical engineering assistant. "
        "Answer questions using the provided document context when available. "
        "Cite sources using [Source N] notation. "
        "If the context doesn't contain relevant information, say so and answer from general knowledge. "
        "Be precise and technical."
    )
    context_text = build_context_prompt(chunks)
    if context_text:
        system_prompt += f"\n\n## Document Context\n\n{context_text}"

    # Get conversation history
    history = (
        db.query(Message)
        .filter(Message.conversation_id == conversation.id)
        .order_by(Message.created_at.desc())
        .limit(CONVERSATION_HISTORY_TURNS * 2 + 1)  # +1 for the current user message
        .all()
    )
    history.reverse()

    # The reply has no length limit, but it shares the context window with the
    # prompt, so half of the window is kept free for it. The system prompt and
    # the new question always go in; earlier messages are added, most recent
    # first, for as long as the prompt stays within the other half.
    prompt_budget = level["ctx"] // 2
    prompt_budget -= _estimate_tokens(system_prompt) + _estimate_tokens(request.message)
    current, earlier = history[-1:], history[:-1]
    kept = []
    for msg in reversed(earlier):
        content = _strip_thinking(msg.content) if msg.role == "assistant" else msg.content
        prompt_budget -= _estimate_tokens(content)
        if prompt_budget < 0:
            logger.info("Chat: dropped %d older message(s) to fit the %d-token context",
                        len(earlier) - len(kept), level["ctx"])
            break
        kept.append({"role": msg.role, "content": content})
    kept.reverse()

    llm_messages = [{"role": "system", "content": system_prompt}] + kept
    for msg in current:
        llm_messages.append({"role": msg.role, "content": msg.content})
    logger.info("Chat: sending %d messages to LLM (depth: %s, context chunks: %d, history: %d)",
                len(llm_messages), level["key"], len(chunks), len(kept))

    # llama-server stops by itself when its context window is full. The MLX
    # server has no fixed window (its memory grows with every token), so there
    # the reply is given what is left of the level's context budget, which is
    # what the memory check covered, and never more than the runtime's crash
    # guard.
    chat_model = llama_supervisor.active_chat_model()
    reply_limit = None
    if chat_model["backend"] == "mlx":
        prompt_tokens = sum(_estimate_tokens(m["content"]) for m in llm_messages)
        reply_limit = max(level["ctx"] - prompt_tokens, 1024)
        if chat_model.get("reply_guard_tokens"):
            reply_limit = min(reply_limit, chat_model["reply_guard_tokens"])

    # Update conversation title from first message
    if len(history) == 1:
        title = request.message[:80] + ("..." if len(request.message) > 80 else "")
        conversation.title = title
        db.commit()

    conv_id = conversation.id
    _chat_conversation_id = conv_id

    async def event_generator():
        global _chat_streaming, _chat_conversation_id, _chat_cancel
        try:
            full_response = []
            llm_result = {}
            try:
                async for token in stream_chat_response(
                        llm_messages, llm_result, max_tokens=reply_limit):
                    if _chat_cancel:
                        break
                    full_response.append(token)
                    yield {"event": "token", "data": json.dumps({"token": token})}
                    # Push to mirror listeners
                    token_evt = {"event": "token", "data": json.dumps({"token": token})}
                    for q in _chat_listeners:
                        q.put_nowait(token_evt)

                # Closing tokens the stream itself did not produce: end a
                # reasoning block left open by Stop, and say so when the reply
                # was cut off rather than finished.
                tail = ""
                so_far = "".join(full_response)
                if so_far.rfind("<think>") > so_far.rfind("</think>"):
                    tail += "</think>\n\n"
                if llm_result.get("finish_reason") == "length":
                    if reply_limit:
                        why = (f"after {reply_limit:,} tokens, the most "
                               f"{chat_model['label']} can safely generate here")
                    else:
                        why = f"because the {level['ctx']:,}-token context window is full"
                    logger.warning("Chat: reply cut off %s", why)
                    tail += f"\n\n*[Response cut off {why}. Send \"continue\" to get the rest.]*"
                if tail:
                    full_response.append(tail)
                    tail_evt = {"event": "token", "data": json.dumps({"token": tail})}
                    yield tail_evt
                    for q in _chat_listeners:
                        q.put_nowait(tail_evt)
            except Exception as e:
                yield {"event": "error", "data": json.dumps({"error": str(e)})}
                error_evt = {"event": "error", "data": json.dumps({"error": str(e)})}
                for q in _chat_listeners:
                    q.put_nowait(error_evt)
                return

            # Save assistant message
            assistant_content = "".join(full_response)
            session = db
            assistant_msg = Message(
                conversation_id=conv_id,
                role="assistant",
                content=assistant_content,
                sources=sources,
            )
            session.add(assistant_msg)
            session.commit()

            done_data = {
                "event": "done",
                "data": json.dumps({
                    "conversation_id": conv_id,
                    "sources": sources,
                }),
            }
            yield done_data
            # Push done to mirror listeners
            for q in _chat_listeners:
                q.put_nowait(done_data)
        finally:
            _chat_streaming = False
            _chat_conversation_id = None
            _chat_cancel = False

    return EventSourceResponse(event_generator())


@router.get("/chat/stream-mirror")
async def chat_stream_mirror():
    """SSE endpoint for mirroring an active stream to other tabs."""
    if not _chat_streaming:
        # Nothing is streaming — return an immediate done so the client closes cleanly
        async def empty():
            yield {"event": "done", "data": json.dumps({"conversation_id": None, "sources": []})}
        return EventSourceResponse(empty())

    queue: asyncio.Queue = asyncio.Queue()
    _chat_listeners.append(queue)

    async def mirror_generator():
        try:
            while True:
                evt = await queue.get()
                yield evt
                if evt.get("event") == "done" or evt.get("event") == "error":
                    break
        finally:
            if queue in _chat_listeners:
                _chat_listeners.remove(queue)

    return EventSourceResponse(mirror_generator())
