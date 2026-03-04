"""FastAPI application entry point."""
from contextlib import asynccontextmanager
from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from starlette.requests import Request

from backend.config import RAG_DIR, DEFAULT_TAGS
from backend.models.database import init_db, SessionLocal
from backend.models.schemas import Tag


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
    yield


app = FastAPI(title="GeoRAG", version="1.0.0", lifespan=lifespan)

# Mount static files
app.mount("/static", StaticFiles(directory=str(RAG_DIR / "frontend" / "static")), name="static")

# Templates
templates = Jinja2Templates(directory=str(RAG_DIR / "frontend" / "templates"))

# Import and include routers
from backend.routers import documents, tags, chat, conversations, processing, explore  # noqa: E402

app.include_router(documents.router, prefix="/api")
app.include_router(tags.router, prefix="/api")
app.include_router(chat.router, prefix="/api")
app.include_router(conversations.router, prefix="/api")
app.include_router(processing.router, prefix="/api")
app.include_router(explore.router, prefix="/api")


@app.get("/")
async def index(request: Request):
    return templates.TemplateResponse("spa.html", {"request": request, "active_page": "chat"})


@app.get("/documents")
async def documents_page(request: Request):
    return templates.TemplateResponse("spa.html", {"request": request, "active_page": "documents"})
