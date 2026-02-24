"""Chat endpoint with SSE streaming."""
import asyncio
import json
from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session
from sse_starlette.sse import EventSourceResponse

from backend.models.database import get_db
from backend.models.schemas import Conversation, Message
from backend.models.pydantic_models import ChatRequest
from backend.services.llm_client import stream_chat_response
from backend.services.vector_store import query_tags
from backend.services.embedding_client import embed_text
from backend.config import MAX_CONTEXT_CHUNKS, CONVERSATION_HISTORY_TURNS

router = APIRouter(tags=["chat"])

_chat_streaming = False
_chat_conversation_id = None
_chat_listeners: list[asyncio.Queue] = []
_chat_cancel = False


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

    # RAG retrieval
    chunks = []
    sources = []
    if request.tag_names:
        try:
            query_embedding = await embed_text(request.message)
            chunks = query_tags(query_embedding, request.tag_names, top_k=request.top_k_per_tag)
            max_ctx = request.max_context_chunks or MAX_CONTEXT_CHUNKS
            chunks = chunks[:max_ctx]
            sources = [
                {"file_path": c.get("file_path", ""), "filename": c.get("filename", ""),
                 "page": c.get("page", ""), "score": c.get("score", 0)}
                for c in chunks
            ]
        except Exception:
            pass  # Continue without RAG context if embedding fails

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

    llm_messages = [{"role": "system", "content": system_prompt}]
    for msg in history:
        llm_messages.append({"role": msg.role, "content": msg.content})

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
            try:
                async for token in stream_chat_response(llm_messages):
                    if _chat_cancel:
                        break
                    full_response.append(token)
                    yield {"event": "token", "data": json.dumps({"token": token})}
                    # Push to mirror listeners
                    token_evt = {"event": "token", "data": json.dumps({"token": token})}
                    for q in _chat_listeners:
                        q.put_nowait(token_evt)
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
