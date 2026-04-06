"""Wiki service — page management, vector ops, and LLM operations."""
import asyncio
import json
import logging
import re
import time
import datetime
from typing import Optional

from sqlalchemy.orm import Session

from backend.config import (
    WIKI_COLLECTION_NAME, WIKI_INGEST_MAX_TOKENS,
    WIKI_INGEST_MAX_SOURCE_CHARS, WIKI_INGEST_MAX_INDEX_CHARS,
    WIKI_QUERY_MAX_CONTEXT_PAGES, WIKI_CHAT_THRESHOLD, WIKI_CHAT_TOP_K,
)
from backend.models.schemas import WikiPage, WikiLog
from backend.models.database import SessionLocal
from backend.services.chunker import chunk_text
from backend.services.embedding_client import embed_text, embed_batch
from backend.services.vector_store import get_or_create_collection, add_chunks, get_file_chunks_any_tag
from backend.services.llm_client import chat_completion, stream_chat_response

logger = logging.getLogger(__name__)

# Wikilink pattern: [[Page Title]]
_WIKILINK_RE = re.compile(r"\[\[(.+?)\]\]")

# ── LLM Prompt Templates ──────────────────────────────────────────────────

WIKI_INGEST_SYSTEM_PROMPT = """\
You are a wiki maintainer for a geotechnical engineering knowledge base.
You are given source material extracted from engineering documents. Each source chunk is annotated as [Source: filename (page N)]. Your job is to:
1. Identify key entities, concepts, and topics in the source material.
2. Create new wiki pages or update existing ones with the extracted knowledge.
3. Use [[Wiki Links]] to cross-reference related pages.
4. Write clear, technical, well-structured markdown content.
5. Include inline citations using [Source: filename (page N)] notation when presenting specific data, values, or findings from the source material.

## Existing Wiki Pages
{index}

## Rules
- Each page should have a clear title and focused content.
- Use categories: entity, concept, topic, source_summary, comparison.
- Cross-reference related pages using [[Page Title]] syntax.
- Include specific data, values, and findings — not vague summaries.
- Cite sources inline: when you include a specific fact, value, or finding, add [Source: filename (page N)] after it.
- If a page already exists on a topic, update it rather than creating a duplicate.
- Include a "source_files" array in each page entry listing which documents were referenced.

Respond with ONLY valid JSON in this format:
{{
  "pages_to_create": [
    {{"title": "...", "category": "...", "content": "...", "summary": "one-line summary",
      "source_files": [{{"filename": "...", "file_path": "...", "pages": ["1", "3"]}}]}}
  ],
  "pages_to_update": [
    {{"slug": "existing-page-slug", "content": "full updated content", "summary": "updated summary",
      "source_files": [{{"filename": "...", "file_path": "...", "pages": ["1", "3"]}}]}}
  ]
}}"""

WIKI_QUERY_SYSTEM_PROMPT = """\
You are GeoRAG Wiki, a geotechnical engineering assistant answering from a curated wiki.
The following wiki pages contain compiled knowledge relevant to the question.
Use them to answer. Cite wiki pages as [Wiki: Page Title].
If the wiki pages don't fully cover the question, say so clearly.
Be precise and technical.

## Wiki Context
{context}"""

WIKI_LINT_SYSTEM_PROMPT = """\
You are a wiki quality checker for a geotechnical engineering knowledge base.
Analyze the wiki pages below and identify issues.

## Wiki Pages
{pages}

Respond with ONLY valid JSON:
{{
  "orphan_pages": ["slugs with no inbound links"],
  "missing_pages": ["titles referenced via [[link]] but no page exists"],
  "stale_pages": ["slugs that may need updating"],
  "missing_crossrefs": [{{"from_slug": "...", "should_link_to": "..."}}],
  "suggested_pages": ["titles for new pages that would fill knowledge gaps"]
}}"""

WIKI_CHAT_GROWTH_PROMPT = """\
You are a wiki maintainer. A user asked a question and the answer was synthesized from raw source documents.
Distill this Q&A exchange into wiki page creates/updates to capture the knowledge for future reference.

## User Question
{question}

## Generated Answer
{answer}

## Source Chunks Used
{sources}

## Existing Wiki Pages
{index}

Rules:
- Only create pages for genuinely useful, reusable knowledge — not trivial Q&A.
- Prefer updating existing pages over creating new ones when the topic overlaps.
- Use [[Wiki Links]] for cross-references.
- Categories: entity, concept, topic, source_summary, comparison.
- Cite sources inline: when you include a specific fact or finding, add [Source: filename (page N)] after it.
- Include a "source_files" array in each page entry listing which documents were referenced.

Respond with ONLY valid JSON:
{{
  "pages_to_create": [
    {{"title": "...", "category": "...", "content": "...", "summary": "one-line summary",
      "source_files": [{{"filename": "...", "file_path": "...", "pages": ["1", "3"]}}]}}
  ],
  "pages_to_update": [
    {{"slug": "existing-page-slug", "content": "full updated content", "summary": "updated summary",
      "source_files": [{{"filename": "...", "file_path": "...", "pages": ["1", "3"]}}]}}
  ],
  "skip_reason": "optional — if no wiki updates are warranted, explain why"
}}"""


# ── Slug Generation ───────────────────────────────────────────────────────

def generate_slug(title: str) -> str:
    """Convert title to URL-safe slug."""
    slug = title.lower().strip()
    slug = re.sub(r"[^\w\s-]", "", slug)
    slug = re.sub(r"[\s_]+", "-", slug)
    slug = re.sub(r"-+", "-", slug).strip("-")
    return slug or "untitled"


def _ensure_unique_slug(db: Session, slug: str) -> str:
    """Ensure slug is unique by appending a number if needed."""
    original = slug
    counter = 1
    while db.query(WikiPage).filter(WikiPage.slug == slug).first():
        slug = f"{original}-{counter}"
        counter += 1
    return slug


# ── Page Management ───────────────────────────────────────────────────────

def get_all_pages(db: Session, category: Optional[str] = None) -> list[WikiPage]:
    """List all wiki pages, optionally filtered by category."""
    query = db.query(WikiPage)
    if category:
        query = query.filter(WikiPage.category == category)
    return query.order_by(WikiPage.updated_at.desc()).all()


def get_page(db: Session, slug: str) -> Optional[WikiPage]:
    """Get a single page by slug."""
    return db.query(WikiPage).filter(WikiPage.slug == slug).first()


def create_page(db: Session, title: str, content: str, category: str = "general",
                summary: Optional[str] = None, source_files: list = None) -> WikiPage:
    """Create a new wiki page."""
    slug = _ensure_unique_slug(db, generate_slug(title))
    page = WikiPage(
        slug=slug,
        title=title,
        content=content,
        category=category,
        summary=summary or "",
        source_files=source_files or [],
    )
    db.add(page)
    db.commit()
    db.refresh(page)
    return page


def update_page(db: Session, slug: str, **fields) -> Optional[WikiPage]:
    """Update a wiki page's fields."""
    page = get_page(db, slug)
    if not page:
        return None
    for key, value in fields.items():
        if value is not None and hasattr(page, key):
            setattr(page, key, value)
    page.updated_at = datetime.datetime.utcnow()
    db.commit()
    db.refresh(page)
    return page


def delete_page(db: Session, slug: str) -> bool:
    """Delete a wiki page."""
    page = get_page(db, slug)
    if not page:
        return False
    # Clean up vector store
    try:
        collection = get_or_create_collection(WIKI_COLLECTION_NAME)
        collection.delete(where={"slug": slug})
    except Exception:
        pass
    db.delete(page)
    db.commit()
    return True


def search_pages(db: Session, query: str) -> list[WikiPage]:
    """Search pages by title and content using SQLite LIKE."""
    pattern = f"%{query}%"
    return (
        db.query(WikiPage)
        .filter((WikiPage.title.ilike(pattern)) | (WikiPage.content.ilike(pattern)))
        .order_by(WikiPage.updated_at.desc())
        .all()
    )


def get_stats(db: Session) -> dict:
    """Get wiki statistics."""
    total = db.query(WikiPage).count()
    categories = {}
    for page in db.query(WikiPage).all():
        categories[page.category] = categories.get(page.category, 0) + 1
    last_updated = (
        db.query(WikiPage)
        .order_by(WikiPage.updated_at.desc())
        .first()
    )
    return {
        "total_pages": total,
        "categories": categories,
        "last_updated": last_updated.updated_at.isoformat() if last_updated else None,
    }


def get_sync_status(db: Session) -> dict:
    """Compute wiki-library sync status for the initialization UI."""
    from backend.models.schemas import File
    from backend.routers.processing import _processor

    processed_count = db.query(File).filter(File.scan_status == "processed").count()
    total_files = db.query(File).count()

    # Last wiki ingest timestamp
    last_ingest_log = (
        db.query(WikiLog)
        .filter(WikiLog.operation == "ingest")
        .order_by(WikiLog.created_at.desc())
        .first()
    )
    last_ingest_at = last_ingest_log.created_at if last_ingest_log else None

    # Count files processed after last ingest
    if last_ingest_at:
        new_since_ingest = db.query(File).filter(
            File.scan_status == "processed",
            File.processed_at > last_ingest_at
        ).count()
    else:
        new_since_ingest = processed_count

    wiki_pages = db.query(WikiPage).count()

    # Compute persistent coverage: which processed files appear in any wiki page's source_files
    all_processed_files = db.query(File).filter(File.scan_status == "processed").all()
    covered_paths = set()
    for page in db.query(WikiPage).all():
        for sf in (page.source_files or []):
            path = sf.get("file_path", "") if isinstance(sf, dict) else str(sf)
            if path:
                covered_paths.add(path)
    covered_count = sum(1 for f in all_processed_files if f.relative_path in covered_paths)
    pending_count = processed_count - covered_count

    return {
        "library_processed_files": processed_count,
        "library_total_files": total_files,
        "library_is_processing": _processor.is_running,
        "wiki_total_pages": wiki_pages,
        "wiki_ever_ingested": last_ingest_at is not None,
        "last_ingest_at": last_ingest_at.isoformat() if last_ingest_at else None,
        "files_since_last_ingest": new_since_ingest,
        "wiki_ingest_running": _ingest_running,
        "wiki_ingest_stopped": _ingest_status.get("was_stopped", False),
        "wiki_covered_files": covered_count,
        "wiki_pending_files": pending_count,
    }


def get_pending_file_ids(db: Session) -> list[int]:
    """Return IDs of processed files not yet covered by any wiki page."""
    from backend.models.schemas import File
    all_processed = db.query(File).filter(File.scan_status == "processed").all()
    covered_paths = set()
    for page in db.query(WikiPage).all():
        for sf in (page.source_files or []):
            path = sf.get("file_path", "") if isinstance(sf, dict) else str(sf)
            if path:
                covered_paths.add(path)
    return [f.id for f in all_processed if f.relative_path not in covered_paths]


# ── Reset Wiki ─────────────────────────────────────────────────────────────

def reset_wiki(db: Session):
    """Delete all wiki pages, logs, vectors, and reset ingest state."""
    # Delete all pages
    db.query(WikiPage).delete()
    db.query(WikiLog).delete()
    db.commit()

    # Clear wiki vector collection
    try:
        from backend.services.vector_store import _client
        _client.delete_collection(WIKI_COLLECTION_NAME)
    except Exception:
        pass

    # Clear ingest state
    _reset_ingest_state()

    logger.info("Wiki reset: all pages, logs, and vectors deleted")


# ── Backlinks ─────────────────────────────────────────────────────────────

def compute_backlinks(db: Session):
    """Scan all pages for [[wikilinks]] and update backlinks columns."""
    pages = db.query(WikiPage).all()
    slug_map = {p.slug: p for p in pages}
    title_to_slug = {p.title.lower(): p.slug for p in pages}

    # Reset all backlinks
    for p in pages:
        p.backlinks = []

    # Build backlink map
    backlink_map: dict[str, list[str]] = {p.slug: [] for p in pages}
    for p in pages:
        links = _WIKILINK_RE.findall(p.content)
        for link_title in links:
            target_slug = title_to_slug.get(link_title.lower()) or generate_slug(link_title)
            if target_slug in backlink_map and target_slug != p.slug:
                if p.slug not in backlink_map[target_slug]:
                    backlink_map[target_slug].append(p.slug)

    # Apply
    for slug, linking_slugs in backlink_map.items():
        if slug in slug_map:
            slug_map[slug].backlinks = linking_slugs

    db.commit()


# ── Wiki Log ──────────────────────────────────────────────────────────────

def add_log_entry(db: Session, operation: str, detail: str, pages_affected: list = None):
    """Append an entry to the wiki log."""
    entry = WikiLog(
        operation=operation,
        detail=detail,
        pages_affected=pages_affected or [],
    )
    db.add(entry)
    db.commit()


def get_log(db: Session, limit: int = 50) -> list[WikiLog]:
    """Get recent wiki log entries."""
    return (
        db.query(WikiLog)
        .order_by(WikiLog.created_at.desc())
        .limit(limit)
        .all()
    )


# ── Index Page ────────────────────────────────────────────────────────────

def build_index_page(db: Session) -> WikiPage:
    """Auto-generate the wiki index page from all pages."""
    pages = get_all_pages(db)

    # Group by category
    by_category: dict[str, list[WikiPage]] = {}
    for p in pages:
        if p.slug == "index":
            continue
        by_category.setdefault(p.category, []).append(p)

    lines = ["# Wiki Index\n"]
    lines.append(f"*{len(pages)} pages total. Auto-generated.*\n")

    for cat in sorted(by_category.keys()):
        cat_pages = sorted(by_category[cat], key=lambda p: p.title)
        lines.append(f"\n## {cat.replace('_', ' ').title()}\n")
        for p in cat_pages:
            summary = f" — {p.summary}" if p.summary else ""
            lines.append(f"- [[{p.title}]]{summary}")

    content = "\n".join(lines)

    # Upsert index page
    index_page = get_page(db, "index")
    if index_page:
        update_page(db, "index", content=content, summary="Auto-generated wiki index")
        return get_page(db, "index")
    else:
        return create_page(db, "Index", content, category="index", summary="Auto-generated wiki index")


def _get_index_text(db: Session) -> str:
    """Get the current index page content, or a message if empty."""
    index_page = get_page(db, "index")
    if index_page:
        return index_page.content
    return "(Wiki is empty — no pages yet)"


# ── Vector Operations ─────────────────────────────────────────────────────

async def embed_wiki_page(page: WikiPage):
    """Chunk and embed a wiki page into the wiki ChromaDB collection."""
    if not page.content.strip():
        return

    chunks = chunk_text(page.content, metadata={"slug": page.slug, "title": page.title})
    if not chunks:
        return

    texts = [c["text"] for c in chunks]
    embeddings = await embed_batch(texts)

    ids = [f"wiki::{page.slug}::chunk_{i}" for i in range(len(chunks))]
    metadatas = [
        {"slug": page.slug, "title": page.title, "category": page.category,
         "chunk_index": i, "file_path": f"wiki/{page.slug}"}
        for i in range(len(chunks))
    ]

    add_chunks(WIKI_COLLECTION_NAME, ids, embeddings, texts, metadatas)
    logger.info("Embedded wiki page '%s': %d chunks", page.slug, len(chunks))


async def remove_wiki_page_vectors(slug: str):
    """Remove all vectors for a wiki page from ChromaDB."""
    try:
        collection = get_or_create_collection(WIKI_COLLECTION_NAME)
        collection.delete(where={"slug": slug})
    except Exception as e:
        logger.warning("Failed to remove vectors for wiki page %s: %s", slug, e)


async def search_wiki_vectors(query_embedding: list[float], top_k: int = 10) -> list[dict]:
    """Search the wiki ChromaDB collection. Returns list of {slug, title, text, score}."""
    try:
        collection = get_or_create_collection(WIKI_COLLECTION_NAME)
        if collection.count() == 0:
            return []

        results = collection.query(
            query_embeddings=[query_embedding],
            n_results=min(top_k, collection.count()),
            include=["documents", "metadatas", "distances"],
        )

        if not results["ids"][0]:
            return []

        hits = []
        seen_slugs = set()
        for idx in range(len(results["ids"][0])):
            metadata = results["metadatas"][0][idx] if results["metadatas"] else {}
            slug = metadata.get("slug", "")
            score = 1 - results["distances"][0][idx]
            # Deduplicate by slug — keep best score per page
            if slug not in seen_slugs:
                seen_slugs.add(slug)
                hits.append({
                    "slug": slug,
                    "title": metadata.get("title", ""),
                    "text": results["documents"][0][idx],
                    "score": score,
                    "category": metadata.get("category", ""),
                })
        return sorted(hits, key=lambda x: x["score"], reverse=True)

    except Exception as e:
        logger.warning("Wiki vector search failed: %s", e)
        return []


async def rebuild_wiki_vectors(db: Session):
    """Re-embed all wiki pages (full reindex)."""
    # Clear existing collection
    try:
        from backend.services.vector_store import _client
        _client.delete_collection(WIKI_COLLECTION_NAME)
    except Exception:
        pass

    pages = get_all_pages(db)
    for page in pages:
        await embed_wiki_page(page)
    logger.info("Rebuilt wiki vectors: %d pages", len(pages))


# ── Chat Integration ──────────────────────────────────────────────────────

async def search_wiki_for_chat(query_embedding: list[float]) -> dict:
    """Search wiki for chat context. Returns {pages, sufficient}."""
    hits = await search_wiki_vectors(query_embedding, top_k=WIKI_CHAT_TOP_K)

    # Filter by threshold
    relevant = [h for h in hits if h["score"] >= WIKI_CHAT_THRESHOLD]

    # Load full page content for relevant hits
    if relevant:
        db = SessionLocal()
        try:
            pages = []
            for hit in relevant:
                page = get_page(db, hit["slug"])
                if page:
                    pages.append({
                        "slug": page.slug,
                        "title": page.title,
                        "content": page.content,
                        "score": hit["score"],
                        "category": page.category,
                    })
            # Consider sufficient if we have at least one highly relevant page
            sufficient = len(pages) > 0 and pages[0]["score"] >= WIKI_CHAT_THRESHOLD
            return {"pages": pages, "sufficient": sufficient}
        finally:
            db.close()

    return {"pages": [], "sufficient": False}


async def grow_wiki_from_chat(question: str, answer: str,
                              source_chunks: list[dict], tag_names: list[str]):
    """Background task: distill a chat Q&A into wiki page updates."""
    db = SessionLocal()
    try:
        logger.info("Wiki growth: distilling chat Q&A into wiki updates")
        t0 = time.time()

        index_text = _get_index_text(db)

        # Format source chunks with page info
        source_text = ""
        file_pages_map: dict[str, dict] = {}  # file_path -> {filename, pages}
        for i, chunk in enumerate(source_chunks[:8], 1):
            source = chunk.get("filename", chunk.get("file_path", "unknown"))
            page = chunk.get("page", "")
            page_str = f" (page {page})" if page else ""
            source_text += f"\n[Source {i}: {source}{page_str}]\n{chunk['text']}\n"

            # Aggregate source files
            fp = chunk.get("file_path", "")
            if fp:
                if fp not in file_pages_map:
                    file_pages_map[fp] = {"filename": chunk.get("filename", ""), "pages": []}
                if page and page not in file_pages_map[fp]["pages"]:
                    file_pages_map[fp]["pages"].append(page)

        chat_source_files = [
            {"file_path": fp, "filename": info["filename"], "pages": info["pages"]}
            for fp, info in file_pages_map.items()
        ]

        prompt = WIKI_CHAT_GROWTH_PROMPT.format(
            question=question,
            answer=answer,
            sources=source_text or "(no source chunks)",
            index=index_text,
        )

        response = await chat_completion(
            [{"role": "system", "content": prompt}],
            max_tokens=WIKI_INGEST_MAX_TOKENS,
        )

        await _apply_llm_wiki_response(db, response, operation="chat_growth",
                                        source_files=chat_source_files)
        logger.info("Wiki growth complete in %.1fs", time.time() - t0)

    except Exception as e:
        logger.warning("Wiki growth failed (best-effort): %s", e)
    finally:
        db.close()


# ── LLM Operations ───────────────────────────────────────────────────────

# Ingest state
_ingest_running = False
_ingest_cancel = False
_ingest_processed_file_ids: set = set()
_ingest_last_tag_names: list = []
_ingest_last_file_ids: list = []
_ingest_status = {
    "is_running": False,
    "phase": "idle",  # idle|ingesting|stopping|stopped|done
    "was_stopped": False,
    "total_sources": 0,
    "processed_sources": 0,
    "pages_created": 0,
    "pages_updated": 0,
    "current_source": None,
    "errors": [],
}


def get_ingest_status() -> dict:
    return dict(_ingest_status)


async def stop_ingest():
    global _ingest_cancel
    _ingest_cancel = True
    _ingest_status["phase"] = "stopping"


def _reset_ingest_state():
    """Clear all ingest resume state."""
    global _ingest_running, _ingest_cancel
    _ingest_running = False
    _ingest_cancel = False
    _ingest_processed_file_ids.clear()
    _ingest_last_tag_names.clear()
    _ingest_last_file_ids.clear()
    _ingest_status.update({
        "is_running": False, "phase": "idle", "was_stopped": False,
        "total_sources": 0, "processed_sources": 0,
        "pages_created": 0, "pages_updated": 0,
        "current_source": None, "errors": [],
    })


async def ingest_sources(tag_names: list[str] = None, file_ids: list[int] = None):
    """Ingest source documents into the wiki via LLM."""
    global _ingest_running, _ingest_cancel, _ingest_status

    if _ingest_running:
        return {"error": "Ingest already running"}

    tag_names = tag_names or []
    file_ids = file_ids or []

    # Detect resume vs fresh run
    is_resume = (
        _ingest_status["was_stopped"]
        and tag_names == _ingest_last_tag_names
        and file_ids == _ingest_last_file_ids
    )

    _ingest_running = True
    _ingest_cancel = False
    _ingest_status["is_running"] = True
    _ingest_status["phase"] = "ingesting"
    _ingest_status["was_stopped"] = False
    _ingest_status["current_source"] = None

    if not is_resume:
        # Fresh run — clear resume state
        _ingest_processed_file_ids.clear()
        _ingest_status.update({
            "total_sources": 0, "processed_sources": 0,
            "pages_created": 0, "pages_updated": 0, "errors": [],
        })

    # Save params for potential resume
    _ingest_last_tag_names[:] = tag_names
    _ingest_last_file_ids[:] = file_ids

    db = SessionLocal()
    try:
        from backend.models.schemas import File, FileTag, Tag

        # Gather source files
        query = db.query(File).filter(File.scan_status == "processed")
        if file_ids:
            query = query.filter(File.id.in_(file_ids))
        elif tag_names:
            query = (
                query.join(FileTag).join(Tag)
                .filter(Tag.name.in_(tag_names))
            )
        all_files = query.all()

        # Filter out already-processed files (for resume)
        files = [f for f in all_files if f.id not in _ingest_processed_file_ids]

        # Set total to reflect overall progress (already done + remaining)
        already_done = len(_ingest_processed_file_ids)
        _ingest_status["total_sources"] = already_done + len(files)
        logger.info("Wiki ingest: %d source files to process (%d already done)",
                     len(files), already_done)

        index_text = _get_index_text(db)

        # Process one file per LLM call to stay within context limits
        for i in range(0, len(files)):
            if _ingest_cancel:
                break

            f = files[i]
            _ingest_status["current_source"] = f.filename

            # Determine which tag collections to search for this file's chunks
            file_tag_names = tag_names if tag_names else [
                ft.tag.name for ft in f.tags if ft.tag
            ]
            chunks = get_file_chunks_any_tag(f.relative_path, file_tag_names)

            source_material = ""
            batch_source_files = []
            if chunks:
                pages_seen = []
                for chunk in chunks:
                    page = chunk.get("page", "")
                    page_str = f" (page {page})" if page else ""
                    addition = f"\n\n[Source: {f.filename}{page_str}]\n{chunk['text']}"
                    if len(source_material) + len(addition) > WIKI_INGEST_MAX_SOURCE_CHARS:
                        break
                    source_material += addition
                    if page and page not in pages_seen:
                        pages_seen.append(page)
                batch_source_files.append({
                    "file_path": f.relative_path,
                    "filename": f.filename,
                    "pages": pages_seen,
                })
            else:
                # Fallback to extracted text preview
                text = f.extracted_text_preview or f.filename
                source_material = f"\n\n[Source: {f.filename}]\n{text}"[:WIKI_INGEST_MAX_SOURCE_CHARS]
                batch_source_files.append({
                    "file_path": f.relative_path,
                    "filename": f.filename,
                    "pages": [],
                })

            # Call LLM
            try:
                capped_index = index_text[:WIKI_INGEST_MAX_INDEX_CHARS]
                prompt = WIKI_INGEST_SYSTEM_PROMPT.format(index=capped_index)
                messages = [
                    {"role": "system", "content": prompt},
                    {"role": "user", "content": f"Process these source documents and create/update wiki pages:\n{source_material}"},
                ]

                response = await chat_completion(messages, max_tokens=WIKI_INGEST_MAX_TOKENS)
                result = await _apply_llm_wiki_response(db, response, operation="ingest",
                                                        source_files=batch_source_files)
                _ingest_status["pages_created"] += result.get("created", 0)
                _ingest_status["pages_updated"] += result.get("updated", 0)

                # Refresh index for next file
                index_text = _get_index_text(db)

            except Exception as e:
                error_msg = f"File {already_done + i + 1}: {str(e)}"
                _ingest_status["errors"].append(error_msg)
                logger.warning("Wiki ingest batch error: %s", e)

            # Track processed file for resume
            _ingest_processed_file_ids.add(f.id)
            _ingest_status["processed_sources"] = len(_ingest_processed_file_ids)

        # Rebuild index page and backlinks
        build_index_page(db)
        compute_backlinks(db)

        add_log_entry(db, "ingest",
                      f"Ingested {_ingest_status['processed_sources']} sources. "
                      f"Created {_ingest_status['pages_created']}, updated {_ingest_status['pages_updated']} pages.",
                      [])

    except Exception as e:
        logger.error("Wiki ingest failed: %s", e)
        _ingest_status["errors"].append(str(e))
    finally:
        db.close()
        was_cancelled = _ingest_cancel
        _ingest_running = False
        _ingest_cancel = False
        _ingest_status["is_running"] = False
        _ingest_status["current_source"] = None
        if was_cancelled:
            _ingest_status["phase"] = "stopped"
            _ingest_status["was_stopped"] = True
            # Keep _ingest_processed_file_ids for resume
        else:
            _ingest_status["phase"] = "done"
            _ingest_status["was_stopped"] = False
            _ingest_processed_file_ids.clear()
            _ingest_last_tag_names.clear()
            _ingest_last_file_ids.clear()


async def query_wiki(db: Session, question: str, save_as_page: bool = False):
    """Query the wiki and stream a response. Returns an async generator of tokens."""
    # Search wiki
    query_embedding = await embed_text(question)
    hits = await search_wiki_vectors(query_embedding, top_k=WIKI_QUERY_MAX_CONTEXT_PAGES)

    # Load full pages
    context_parts = []
    pages_used = []
    for hit in hits:
        page = get_page(db, hit["slug"])
        if page:
            context_parts.append(f"### [[{page.title}]]\n*Category: {page.category}*\n\n{page.content}")
            pages_used.append({"slug": page.slug, "title": page.title, "score": hit["score"]})

    context = "\n\n---\n\n".join(context_parts) if context_parts else "(No relevant wiki pages found)"

    system_prompt = WIKI_QUERY_SYSTEM_PROMPT.format(context=context)
    messages = [
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": question},
    ]

    full_response = []
    async for token in stream_chat_response(messages):
        full_response.append(token)
        yield {"event": "token", "token": token}

    answer = "".join(full_response)

    # Optionally save as wiki page
    saved_slug = None
    if save_as_page and answer.strip():
        title = question[:80] + ("..." if len(question) > 80 else "")
        transitive_sources = _collect_transitive_sources(db, pages_used)
        page = create_page(db, title, answer, category="topic",
                           summary=question[:200], source_files=transitive_sources)
        await embed_wiki_page(page)
        saved_slug = page.slug

        build_index_page(db)
        compute_backlinks(db)

    add_log_entry(db, "query", f"Q: {question[:100]}", [s["slug"] for s in pages_used])

    yield {
        "event": "done",
        "pages_used": pages_used,
        "saved_slug": saved_slug,
    }


async def lint_wiki(db: Session) -> dict:
    """Run a wiki health check via LLM."""
    pages = get_all_pages(db)
    if not pages:
        return {"error": "Wiki is empty — nothing to lint"}

    # Build pages summary for LLM
    pages_text = ""
    for p in pages:
        links = _WIKILINK_RE.findall(p.content)
        pages_text += (
            f"\n### {p.title} (slug: {p.slug}, category: {p.category})\n"
            f"Summary: {p.summary or 'none'}\n"
            f"Outbound links: {', '.join(links) if links else 'none'}\n"
            f"Backlinks: {', '.join(p.backlinks) if p.backlinks else 'none'}\n"
        )

    prompt = WIKI_LINT_SYSTEM_PROMPT.format(pages=pages_text)
    response = await chat_completion(
        [{"role": "system", "content": prompt}],
        max_tokens=2048,
    )

    try:
        result = _parse_json_response(response)
    except Exception:
        result = {"raw_response": response, "error": "Failed to parse lint results"}

    add_log_entry(db, "lint", f"Lint completed. Found issues in response.", [])
    return result


# ── Internal Helpers ──────────────────────────────────────────────────────

def _parse_json_response(text: str) -> dict:
    """Extract JSON from LLM response (may be wrapped in markdown code blocks)."""
    # Try direct parse
    text = text.strip()
    if text.startswith("{"):
        return json.loads(text)
    # Try extracting from code block
    match = re.search(r"```(?:json)?\s*\n?(.*?)\n?```", text, re.DOTALL)
    if match:
        return json.loads(match.group(1).strip())
    # Last resort: find first { to last }
    start = text.find("{")
    end = text.rfind("}") + 1
    if start >= 0 and end > start:
        return json.loads(text[start:end])
    raise ValueError(f"No JSON found in response: {text[:200]}")


def _merge_source_files(existing: list, new: list) -> list:
    """Merge two source_files lists, deduplicating by file_path and merging pages."""
    by_path: dict[str, dict] = {}
    for sf in (existing or []) + (new or []):
        if not isinstance(sf, dict):
            continue
        fp = sf.get("file_path", "")
        if not fp:
            continue
        if fp not in by_path:
            by_path[fp] = {"file_path": fp, "filename": sf.get("filename", ""), "pages": []}
        for p in sf.get("pages", []):
            if p and p not in by_path[fp]["pages"]:
                by_path[fp]["pages"].append(p)
    return list(by_path.values())


def _collect_transitive_sources(db: Session, pages_used: list[dict]) -> list[dict]:
    """Collect and merge source_files from wiki pages used in a query (transitive references)."""
    all_sources: list[dict] = []
    for pu in pages_used:
        page = get_page(db, pu["slug"])
        if page and page.source_files:
            all_sources.extend(page.source_files)
    return _merge_source_files([], all_sources)


async def _apply_llm_wiki_response(db: Session, response: str, operation: str,
                                    source_files: list[dict] = None) -> dict:
    """Parse LLM JSON response and create/update wiki pages."""
    try:
        data = _parse_json_response(response)
    except Exception as e:
        logger.warning("Failed to parse LLM wiki response: %s", e)
        return {"created": 0, "updated": 0, "error": str(e)}

    # Check if LLM decided to skip
    if data.get("skip_reason"):
        logger.info("Wiki %s skipped: %s", operation, data["skip_reason"])
        return {"created": 0, "updated": 0, "skipped": data["skip_reason"]}

    created = 0
    updated = 0
    affected_slugs = []

    # Create new pages
    for page_data in data.get("pages_to_create", []):
        try:
            title = page_data.get("title", "").strip()
            if not title:
                continue
            # Use per-page source_files from LLM if present, else batch-level fallback
            page_sources = page_data.get("source_files") or source_files or []
            page = create_page(
                db,
                title=title,
                content=page_data.get("content", ""),
                category=page_data.get("category", "general"),
                summary=page_data.get("summary", ""),
                source_files=page_sources,
            )
            await embed_wiki_page(page)
            affected_slugs.append(page.slug)
            created += 1
            logger.info("Wiki %s: created page '%s'", operation, page.slug)
        except Exception as e:
            logger.warning("Wiki %s: failed to create page '%s': %s",
                           operation, page_data.get("title", "?"), e)

    # Update existing pages
    for page_data in data.get("pages_to_update", []):
        try:
            slug = page_data.get("slug", "").strip()
            if not slug:
                continue
            # Merge new sources with existing page sources
            new_sources = page_data.get("source_files") or source_files or []
            existing_page = get_page(db, slug)
            merged_sources = _merge_source_files(
                existing_page.source_files if existing_page else [],
                new_sources,
            )
            page = update_page(
                db, slug,
                content=page_data.get("content"),
                summary=page_data.get("summary"),
                source_files=merged_sources,
            )
            if page:
                await remove_wiki_page_vectors(slug)
                await embed_wiki_page(page)
                affected_slugs.append(slug)
                updated += 1
                logger.info("Wiki %s: updated page '%s'", operation, slug)
        except Exception as e:
            logger.warning("Wiki %s: failed to update page '%s': %s",
                           operation, page_data.get("slug", "?"), e)

    if affected_slugs:
        compute_backlinks(db)
        build_index_page(db)
        add_log_entry(db, operation,
                      f"Created {created}, updated {updated} pages",
                      affected_slugs)

    return {"created": created, "updated": updated}
