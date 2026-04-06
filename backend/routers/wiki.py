"""Wiki API endpoints."""
import asyncio
import json
import logging
from typing import Optional

from fastapi import APIRouter, Depends, Query, HTTPException
from sqlalchemy.orm import Session
from sse_starlette.sse import EventSourceResponse

from backend.models.database import get_db
from backend.models.pydantic_models import (
    WikiPageCreate, WikiPageUpdate, WikiPageResponse,
    WikiIngestRequest, WikiQueryRequest, WikiLogResponse,
    WikiLintFixRequest,
)
from backend.services import wiki_service

logger = logging.getLogger(__name__)

router = APIRouter(tags=["wiki"])


# ── Page CRUD ─────────────────────────────────────────────────────────────

@router.get("/wiki/pages")
async def list_pages(category: Optional[str] = None, db: Session = Depends(get_db)):
    """List all wiki pages, optionally filtered by category."""
    pages = wiki_service.get_all_pages(db, category=category)
    return [
        {
            "id": p.id, "slug": p.slug, "title": p.title,
            "category": p.category, "summary": p.summary,
            "backlinks": p.backlinks or [],
            "source_files": p.source_files or [],
            "created_at": p.created_at.isoformat() if p.created_at else None,
            "updated_at": p.updated_at.isoformat() if p.updated_at else None,
        }
        for p in pages
    ]


@router.get("/wiki/pages/{slug}")
async def get_page(slug: str, db: Session = Depends(get_db)):
    """Get a single wiki page by slug."""
    page = wiki_service.get_page(db, slug)
    if not page:
        raise HTTPException(status_code=404, detail="Page not found")
    return {
        "id": page.id, "slug": page.slug, "title": page.title,
        "content": page.content, "category": page.category,
        "summary": page.summary, "backlinks": page.backlinks or [],
        "source_files": page.source_files or [],
        "created_at": page.created_at.isoformat() if page.created_at else None,
        "updated_at": page.updated_at.isoformat() if page.updated_at else None,
    }


@router.post("/wiki/pages")
async def create_page(request: WikiPageCreate, db: Session = Depends(get_db)):
    """Create a new wiki page."""
    page = wiki_service.create_page(
        db, title=request.title, content=request.content,
        category=request.category, summary=request.summary,
        source_files=request.source_files,
    )
    # Embed in background
    await wiki_service.embed_wiki_page(page)
    wiki_service.build_index_page(db)
    wiki_service.compute_backlinks(db)
    return {"slug": page.slug, "id": page.id}


@router.put("/wiki/pages/{slug}")
async def update_page(slug: str, request: WikiPageUpdate, db: Session = Depends(get_db)):
    """Update a wiki page."""
    fields = request.model_dump(exclude_none=True)
    if not fields:
        raise HTTPException(status_code=400, detail="No fields to update")
    page = wiki_service.update_page(db, slug, **fields)
    if not page:
        raise HTTPException(status_code=404, detail="Page not found")
    # Re-embed
    await wiki_service.remove_wiki_page_vectors(slug)
    await wiki_service.embed_wiki_page(page)
    wiki_service.build_index_page(db)
    wiki_service.compute_backlinks(db)
    return {"slug": page.slug, "ok": True}


@router.delete("/wiki/pages/{slug}")
async def delete_page(slug: str, db: Session = Depends(get_db)):
    """Delete a wiki page."""
    ok = wiki_service.delete_page(db, slug)
    if not ok:
        raise HTTPException(status_code=404, detail="Page not found")
    wiki_service.build_index_page(db)
    wiki_service.compute_backlinks(db)
    return {"ok": True}


# ── Search ────────────────────────────────────────────────────────────────

@router.get("/wiki/search")
async def search_wiki(q: str = Query(..., min_length=1), db: Session = Depends(get_db)):
    """Hybrid search: SQLite LIKE + vector similarity."""
    # SQLite text search
    text_results = wiki_service.search_pages(db, q)
    text_slugs = {p.slug for p in text_results}

    # Vector search
    vector_results = []
    try:
        from backend.services.embedding_client import embed_text
        embedding = await embed_text(q)
        vector_results = await wiki_service.search_wiki_vectors(embedding, top_k=10)
    except Exception as e:
        logger.warning("Wiki vector search failed: %s", e)

    # Merge results
    results = []
    seen = set()

    # Text matches first
    for p in text_results:
        seen.add(p.slug)
        # Find vector score if available
        score = 0
        for vr in vector_results:
            if vr["slug"] == p.slug:
                score = vr["score"]
                break
        results.append({
            "slug": p.slug, "title": p.title, "category": p.category,
            "summary": p.summary, "score": score,
            "match_type": "text" if score == 0 else "both",
        })

    # Vector-only matches
    for vr in vector_results:
        if vr["slug"] not in seen:
            seen.add(vr["slug"])
            results.append({
                "slug": vr["slug"], "title": vr["title"], "category": vr["category"],
                "summary": "", "score": vr["score"],
                "match_type": "vector",
            })

    return results


# ── Ingest ────────────────────────────────────────────────────────────────

@router.post("/wiki/ingest")
async def start_ingest(request: WikiIngestRequest):
    """Start wiki ingest from source documents. Runs in background."""
    status = wiki_service.get_ingest_status()
    if status["is_running"]:
        raise HTTPException(status_code=409, detail="Ingest already running")

    # Run in background — ingest_sources creates its own session
    asyncio.create_task(
        wiki_service.ingest_sources(tag_names=request.tag_names, file_ids=request.file_ids)
    )
    return {"ok": True, "message": "Ingest started"}


@router.get("/wiki/ingest/status")
async def ingest_status():
    """Get current ingest operation status."""
    return wiki_service.get_ingest_status()


@router.post("/wiki/ingest/stop")
async def stop_ingest():
    """Stop running ingest."""
    await wiki_service.stop_ingest()
    return {"ok": True}


@router.post("/wiki/ingest/pending")
async def start_pending_ingest(db: Session = Depends(get_db)):
    """Start ingest for processed files not yet covered by any wiki page."""
    status = wiki_service.get_ingest_status()
    if status["is_running"]:
        raise HTTPException(status_code=409, detail="Ingest already running")
    pending_ids = wiki_service.get_pending_file_ids(db)
    if not pending_ids:
        return {"ok": True, "message": "No pending files", "count": 0}
    asyncio.create_task(wiki_service.ingest_sources(file_ids=pending_ids))
    return {"ok": True, "message": f"Ingest started for {len(pending_ids)} pending files", "count": len(pending_ids)}


# ── Query ─────────────────────────────────────────────────────────────────

@router.post("/wiki/query")
async def query_wiki(request: WikiQueryRequest, db: Session = Depends(get_db)):
    """Query the wiki with streaming response."""
    async def event_generator():
        try:
            async for event in wiki_service.query_wiki(db, request.question, request.save_as_page):
                if event.get("event") == "token":
                    yield {"event": "token", "data": json.dumps({"token": event["token"]})}
                elif event.get("event") == "done":
                    yield {
                        "event": "done",
                        "data": json.dumps({
                            "pages_used": event.get("pages_used", []),
                            "saved_slug": event.get("saved_slug"),
                        }),
                    }
        except Exception as e:
            yield {"event": "error", "data": json.dumps({"error": str(e)})}

    return EventSourceResponse(event_generator())


# ── Lint ──────────────────────────────────────────────────────────────────

@router.post("/wiki/lint")
async def lint_wiki(db: Session = Depends(get_db)):
    """Run wiki health check."""
    return await wiki_service.lint_wiki(db)


@router.post("/wiki/lint/apply")
async def apply_lint_fixes(request: WikiLintFixRequest):
    """Apply selected health check fixes. Runs as background task."""
    status = wiki_service.get_lint_fix_status()
    if status["is_running"]:
        raise HTTPException(status_code=409, detail="Lint fix already running")
    if wiki_service._ingest_running:
        raise HTTPException(status_code=409, detail="Wiki ingest is running — try again later")
    asyncio.create_task(wiki_service.apply_lint_fixes(request.model_dump()))
    return {"ok": True, "message": "Lint fix started"}


@router.get("/wiki/lint/apply/status")
async def lint_fix_status():
    """Get current lint fix operation status."""
    return wiki_service.get_lint_fix_status()


@router.post("/wiki/lint/apply/stop")
async def stop_lint_fix():
    """Stop running lint fix."""
    await wiki_service.stop_lint_fix()
    return {"ok": True}


# ── Log ───────────────────────────────────────────────────────────────────

@router.get("/wiki/log")
async def get_log(limit: int = 50, db: Session = Depends(get_db)):
    """Get wiki operation log."""
    entries = wiki_service.get_log(db, limit=limit)
    return [
        {
            "id": e.id, "operation": e.operation, "detail": e.detail,
            "pages_affected": e.pages_affected or [],
            "created_at": e.created_at.isoformat() if e.created_at else None,
        }
        for e in entries
    ]


# ── Stats ─────────────────────────────────────────────────────────────────

@router.get("/wiki/stats")
async def get_stats(db: Session = Depends(get_db)):
    """Get wiki statistics including library sync status."""
    stats = wiki_service.get_stats(db)
    sync = wiki_service.get_sync_status(db)
    return {**stats, **sync}


# ── Reset ─────────────────────────────────────────────────────────────────

@router.delete("/wiki/reset")
async def reset_wiki(db: Session = Depends(get_db)):
    """Delete all wiki pages, logs, and vectors. Resets wiki to uninitialized state."""
    wiki_service.reset_wiki(db)
    return {"ok": True}
