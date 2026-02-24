"""Chat endpoint with SSE streaming."""
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


@router.post("/chat")
async def chat(request: ChatRequest, db: Session = Depends(get_db)):
    # Get or create conversation
    if request.conversation_id:
        conversation = db.query(Conversation).get(request.conversation_id)
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
            chunks = query_tags(query_embedding, request.tag_names)
            chunks = chunks[:MAX_CONTEXT_CHUNKS]
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

    async def event_generator():
        full_response = []
        try:
            async for token in stream_chat_response(llm_messages):
                full_response.append(token)
                yield {"event": "token", "data": json.dumps({"token": token})}
        except Exception as e:
            yield {"event": "error", "data": json.dumps({"error": str(e)})}
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

        yield {
            "event": "done",
            "data": json.dumps({
                "conversation_id": conv_id,
                "sources": sources,
            }),
        }

    return EventSourceResponse(event_generator())
