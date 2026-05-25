"""Chat endpoint with SSE streaming."""
import asyncio
import json
import logging
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
from backend.services import wiki_service, llama_supervisor
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
    # llama-servers may be paused (manual auto-shutdown or end of downtime
    # before resume). Start them on demand; the status pill will reflect
    # "Starting LLMs" while we wait.
    ready = await llama_supervisor.ensure_running()
    if not ready:
        return EventSourceResponse(
            iter([{"event": "error", "data": json.dumps({"error": "Local LLM servers did not start in time. Try again in a moment."})}])
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

    # Two-tier retrieval: Wiki-first, then RAG fallback
    chunks = []
    sources = []
    wiki_pages_used = []
    context_source = "none"  # "wiki", "rag", or "hybrid"
    query_embedding = None

    # Step 1: Embed query (needed for both wiki and RAG search)
    try:
        logger.info("Chat: embedding query (%d chars)", len(request.message))
        t0 = time.time()
        query_embedding = await embed_text(request.message)
        logger.info("Chat: query embedded in %.1fs", time.time() - t0)
    except Exception as e:
        logger.warning("Chat: embedding failed: %s", e)

    # Step 2: Wiki-first search
    wiki_context_text = ""
    if query_embedding:
        try:
            wiki_result = await wiki_service.search_wiki_for_chat(query_embedding)
            if wiki_result["pages"]:
                wiki_pages_used = [
                    {"slug": p["slug"], "title": p["title"], "score": p["score"]}
                    for p in wiki_result["pages"]
                ]
                wiki_parts = []
                for p in wiki_result["pages"]:
                    wiki_parts.append(f"[Wiki: {p['title']}]\n{p['content']}")
                wiki_context_text = "\n\n---\n\n".join(wiki_parts)
                if wiki_result["sufficient"]:
                    context_source = "wiki"
                    logger.info("Chat: wiki sufficient — %d pages, best score %.3f",
                                len(wiki_pages_used), wiki_pages_used[0]["score"])
        except Exception as e:
            logger.warning("Chat: wiki search failed (will fall back to RAG): %s", e)

    # Step 3: RAG fallback (if wiki insufficient and tags are selected)
    if context_source != "wiki" and request.tag_names and query_embedding:
        try:
            t0 = time.time()
            chunks = query_tags(query_embedding, request.tag_names, top_k=request.top_k_per_tag)
            max_ctx = request.max_context_chunks or MAX_CONTEXT_CHUNKS
            chunks = chunks[:max_ctx]
            sources = [
                {"file_path": c.get("file_path", ""), "filename": c.get("filename", ""),
                 "page": c.get("page", ""), "score": c.get("score", 0)}
                for c in chunks
            ]
            context_source = "hybrid" if wiki_context_text else "rag"
            logger.info("Chat RAG: retrieved %d chunks in %.1fs (mode: %s)",
                        len(chunks), time.time() - t0, context_source)
        except Exception as e:
            logger.warning("Chat RAG: retrieval failed: %s", e)
            if wiki_context_text:
                context_source = "wiki"

    # Build messages for LLM
    if context_source == "wiki":
        system_prompt = (
            "You are GeoRAG, a geotechnical engineering assistant. "
            "The following wiki pages contain compiled knowledge relevant to the question. "
            "Use them to answer. Cite wiki pages using [Wiki: Page Title] notation. "
            "If the wiki pages don't fully cover the question, say so and answer from general knowledge. "
            "Be precise and technical."
            f"\n\n## Wiki Context\n\n{wiki_context_text}"
        )
    elif context_source == "hybrid":
        rag_context = build_context_prompt(chunks)
        system_prompt = (
            "You are GeoRAG, a geotechnical engineering assistant. "
            "You have two knowledge sources: compiled Wiki pages and raw Document chunks. "
            "Prefer wiki knowledge when available — it's curated and reliable. "
            "Supplement with document context where the wiki has gaps. "
            "Cite wiki pages as [Wiki: Page Title] and documents as [Source N]. "
            "Be precise and technical."
            f"\n\n## Wiki Context\n\n{wiki_context_text}"
            f"\n\n## Document Context\n\n{rag_context}"
        )
    else:
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
    logger.info("Chat: sending %d messages to LLM (context chunks: %d, history: %d)",
                len(llm_messages), len(chunks), len(history))

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
                    "context_source": context_source,
                    "wiki_pages_used": wiki_pages_used,
                }),
            }
            yield done_data
            # Push done to mirror listeners
            for q in _chat_listeners:
                q.put_nowait(done_data)

            # Wiki growth: after RAG/hybrid answers, grow the wiki in the background
            if context_source in ("rag", "hybrid") and assistant_content.strip() and chunks:
                try:
                    asyncio.create_task(
                        wiki_service.grow_wiki_from_chat(
                            request.message, assistant_content, chunks, request.tag_names or []
                        )
                    )
                    logger.info("Chat: wiki growth task started in background")
                except Exception as e:
                    logger.warning("Chat: wiki growth task failed to start: %s", e)
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
