"""FastAPI application entry point."""
import logging
from contextlib import asynccontextmanager
from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from starlette.requests import Request

from backend.config import RAG_DIR, BUNDLE_DIR, DEFAULT_TAGS
from backend.models.database import init_db, SessionLocal
from backend.models.schemas import Tag

# Configure logging for backend modules
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)-7s [%(name)s] %(message)s",
    datefmt="%H:%M:%S",
)
# Keep third-party loggers quiet
logging.getLogger("httpx").setLevel(logging.WARNING)
logging.getLogger("httpcore").setLevel(logging.WARNING)
logging.getLogger("chromadb").setLevel(logging.WARNING)
# ChromaDB ships a PostHog client whose API drifted from the installed
# posthog package, so it logs ERROR per request even though
# ANONYMIZED_TELEMETRY=False suppresses the actual send. Mute it.
logging.getLogger("chromadb.telemetry.product.posthog").setLevel(logging.CRITICAL)
logging.getLogger("uvicorn.access").setLevel(logging.WARNING)
# python-multipart warns about benign trailing bytes after the final boundary
# on every browser/curl upload; raise the threshold so only real errors show.
logging.getLogger("python_multipart.multipart").setLevel(logging.ERROR)


def seed_tags():
    """Seed default tags if they don't exist."""
    db = SessionLocal()
    try:
        existing = db.query(Tag).count()
        if existing == 0:
            for tag_data in DEFAULT_TAGS:
                db.add(Tag(**tag_data))
            db.commit()
    finally:
        db.close()


@asynccontextmanager
async def lifespan(app: FastAPI):
    init_db()
    seed_tags()

    # Lazy imports so circular deps stay shallow
    from backend.services import llama_supervisor, scheduler

    settings = scheduler.get_settings()
    if scheduler.is_in_downtime(settings):
        logging.getLogger(__name__).info(
            "Starting inside downtime window — skipping llama-server launch")
    else:
        # Fire-and-wait: do not block startup if it takes >180s, but log loudly.
        try:
            ok = await llama_supervisor.start_all(wait=True, timeout=180.0)
            if not ok:
                logging.getLogger(__name__).error(
                    "llama-servers did not become ready during startup; "
                    "the app will continue, but chat/embeddings will fail until they recover")
        except Exception:
            logging.getLogger(__name__).exception("llama-server startup failed")

    scheduler.start_scheduler()

    try:
        yield
    finally:
        await scheduler.stop_scheduler()
        # Note: we intentionally do NOT stop llama-server on app shutdown.
        # The pid files let a fresh app instance adopt them, which is friendlier
        # during dev (uvicorn --reload) and avoids losing the warm KV cache.


app = FastAPI(title="GeoRAG", version="1.0.0", lifespan=lifespan)

# Mount static files (BUNDLE_DIR points to _MEIPASS when running as EXE)
app.mount("/static", StaticFiles(directory=str(BUNDLE_DIR / "frontend" / "static")), name="static")

# Templates
templates = Jinja2Templates(directory=str(BUNDLE_DIR / "frontend" / "templates"))

# Import and include routers
from backend.routers import (  # noqa: E402
    documents, tags, chat, conversations, processing, explore, ocr,
    health, wiki, backup, system,
)

app.include_router(documents.router, prefix="/api")
app.include_router(tags.router, prefix="/api")
app.include_router(chat.router, prefix="/api")
app.include_router(conversations.router, prefix="/api")
app.include_router(processing.router, prefix="/api")
app.include_router(explore.router, prefix="/api")
app.include_router(ocr.router, prefix="/api")
app.include_router(health.router, prefix="/api")
app.include_router(wiki.router, prefix="/api")
app.include_router(backup.router, prefix="/api")
app.include_router(system.router, prefix="/api")


@app.get("/")
async def index(request: Request):
    return templates.TemplateResponse("spa.html", {"request": request, "active_page": "chat"})


@app.get("/documents")
async def documents_page(request: Request):
    return templates.TemplateResponse("spa.html", {"request": request, "active_page": "documents"})


@app.get("/wiki")
async def wiki_page(request: Request):
    return templates.TemplateResponse("spa.html", {"request": request, "active_page": "wiki"})
