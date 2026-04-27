"""Export/import (backup/restore) service for GeoRAG.

Produces a cross-platform .georag archive (ZIP) containing:
  manifest.json        - version, platform, embedding model, stats
  files.jsonl          - file metadata rows
  tags.json            - tag definitions
  file_tags.jsonl      - file-tag associations
  wiki_pages.jsonl     - wiki page rows
  wiki_log.jsonl       - operation log
  vectors/
    tag_<name>.npz     - per-collection embeddings (float32 numpy)
    wiki.npz           - wiki page vectors
"""
import asyncio
import datetime
import io
import json
import logging
import platform
import shutil
import tempfile
import zipfile
from pathlib import Path
from typing import Optional

import numpy as np
from sqlalchemy.orm import Session

from backend.config import (
    DATA_DIR, EMBEDDING_MODEL, EMBEDDING_DIM,
    CHUNK_SIZE, CHUNK_OVERLAP, WIKI_COLLECTION_NAME, METADATA_DB,
)
from backend.models.database import SessionLocal
from backend.models.schemas import (
    File, Tag, FileTag, WikiPage, WikiLog, Conversation, Message,
)
from backend.services.vector_store import (
    _client as chroma_client,
    get_or_create_collection, add_chunks,
)

logger = logging.getLogger(__name__)

EXPORT_DIR = DATA_DIR / "exports"
FORMAT_VERSION = 1
CHROMA_BATCH_SIZE = 10_000


# ── Helpers ──────────────────────────────────────────────────────────────────

def _serialize_datetime(obj):
    """JSON serializer for datetime objects."""
    if isinstance(obj, datetime.datetime):
        return obj.isoformat()
    raise TypeError(f"Not serializable: {type(obj)}")


def _parse_datetime(s: Optional[str]) -> Optional[datetime.datetime]:
    """Parse an ISO datetime string back to a datetime object."""
    if not s:
        return None
    return datetime.datetime.fromisoformat(s)


def _row_to_dict(row, columns: list[str]) -> dict:
    """Convert a SQLAlchemy model instance to a dict of the given columns."""
    d = {}
    for col in columns:
        val = getattr(row, col, None)
        if isinstance(val, datetime.datetime):
            val = val.isoformat()
        d[col] = val
    return d


# ── Exporter ─────────────────────────────────────────────────────────────────

class BackupExporter:
    def __init__(self):
        self.is_running = False
        self.phase = "idle"
        self.current_collection = ""
        self.collections_done = 0
        self.collections_total = 0
        self.chunks_exported = 0
        self.error = None
        self._cancel = False
        self._output_path: Optional[Path] = None

    def get_status(self) -> dict:
        return {
            "is_running": self.is_running,
            "phase": self.phase,
            "current_collection": self.current_collection,
            "collections_done": self.collections_done,
            "collections_total": self.collections_total,
            "chunks_exported": self.chunks_exported,
            "error": self.error,
        }

    def stop(self):
        self._cancel = True

    async def run(self, include_wiki: bool = True, include_rag: bool = True):
        if self.is_running:
            return
        self.is_running = True
        self.phase = "idle"
        self.current_collection = ""
        self.collections_done = 0
        self.collections_total = 0
        self.chunks_exported = 0
        self.error = None
        self._cancel = False
        self._output_path = None

        tmp_dir = None
        try:
            EXPORT_DIR.mkdir(parents=True, exist_ok=True)
            tmp_dir = Path(tempfile.mkdtemp(prefix="georag_export_"))

            db = SessionLocal()
            stats = {}
            try:
                # ── Phase: sqlite ──
                self.phase = "sqlite"
                await asyncio.sleep(0)

                stats = await self._export_sqlite(db, tmp_dir, include_wiki)
            finally:
                db.close()

            if self._cancel:
                return

            # ── Phase: vectors ──
            self.phase = "vectors"
            await self._export_vectors(tmp_dir, include_wiki, include_rag, stats)

            if self._cancel:
                return

            # ── Phase: packaging ──
            self.phase = "packaging"
            await asyncio.sleep(0)

            # Write manifest
            manifest = {
                "format_version": FORMAT_VERSION,
                "created_at": datetime.datetime.utcnow().isoformat(),
                "source_platform": f"{platform.system().lower()}-{platform.machine()}",
                "embedding_model": EMBEDDING_MODEL,
                "embedding_dim": EMBEDDING_DIM,
                "chunk_size": CHUNK_SIZE,
                "chunk_overlap": CHUNK_OVERLAP,
                "stats": stats,
                "includes": {
                    "wiki": include_wiki,
                    "rag": include_rag,
                },
            }
            (tmp_dir / "manifest.json").write_text(
                json.dumps(manifest, indent=2), encoding="utf-8"
            )

            # Package into ZIP
            timestamp = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")
            out_path = EXPORT_DIR / f"backup_{timestamp}.georag"
            await asyncio.to_thread(self._build_zip, tmp_dir, out_path)

            self._output_path = out_path
            self.phase = "done"
            logger.info("Backup export complete: %s (%.1f MB)",
                        out_path.name, out_path.stat().st_size / 1_048_576)

        except Exception as e:
            logger.error("Backup export failed: %s", e)
            self.error = str(e)
            self.phase = "error"
        finally:
            self.is_running = False
            if tmp_dir and tmp_dir.exists():
                shutil.rmtree(tmp_dir, ignore_errors=True)

    async def _export_sqlite(self, db: Session, tmp_dir: Path,
                             include_wiki: bool) -> dict:
        """Export SQLite tables to JSONL files. Returns stats dict."""
        stats = {}

        # Files
        file_cols = [
            "id", "relative_path", "filename", "extension", "size_bytes",
            "modified_time", "parent_directory", "content_hash", "scan_status",
            "extracted_text_preview", "chunk_count", "processed_at",
            "auto_tagged", "auto_tag_confidence",
        ]
        count = 0
        with open(tmp_dir / "files.jsonl", "w", encoding="utf-8") as f:
            for row in db.query(File).yield_per(1000):
                f.write(json.dumps(_row_to_dict(row, file_cols),
                                   default=_serialize_datetime) + "\n")
                count += 1
        stats["files"] = count

        # Tags
        tags = []
        for row in db.query(Tag).all():
            tags.append(_row_to_dict(row, [
                "id", "name", "display_name", "description", "color", "file_count",
            ]))
        (tmp_dir / "tags.json").write_text(
            json.dumps(tags, indent=2), encoding="utf-8"
        )
        stats["tags"] = len(tags)

        # FileTags
        count = 0
        with open(tmp_dir / "file_tags.jsonl", "w", encoding="utf-8") as f:
            for row in db.query(FileTag).yield_per(1000):
                f.write(json.dumps(_row_to_dict(row, [
                    "id", "file_id", "tag_id", "source", "confidence", "created_at",
                ]), default=_serialize_datetime) + "\n")
                count += 1
        stats["file_tags"] = count

        # Wiki
        if include_wiki:
            count = 0
            with open(tmp_dir / "wiki_pages.jsonl", "w", encoding="utf-8") as f:
                for row in db.query(WikiPage).yield_per(500):
                    f.write(json.dumps(_row_to_dict(row, [
                        "id", "slug", "title", "content", "category", "summary",
                        "source_files", "backlinks", "created_at", "updated_at",
                    ]), default=_serialize_datetime) + "\n")
                    count += 1
            stats["wiki_pages"] = count

            count = 0
            with open(tmp_dir / "wiki_log.jsonl", "w", encoding="utf-8") as f:
                for row in db.query(WikiLog).yield_per(500):
                    f.write(json.dumps(_row_to_dict(row, [
                        "id", "operation", "detail", "pages_affected", "created_at",
                    ]), default=_serialize_datetime) + "\n")
                    count += 1
            stats["wiki_log_entries"] = count

        return stats

    async def _export_vectors(self, tmp_dir: Path, include_wiki: bool,
                              include_rag: bool, stats: dict):
        """Export ChromaDB collections to .npz files."""
        vectors_dir = tmp_dir / "vectors"
        vectors_dir.mkdir()

        collections = chroma_client.list_collections()
        # Filter by what we want to export
        target_collections = []
        for col in collections:
            name = col if isinstance(col, str) else col.name
            if name == WIKI_COLLECTION_NAME and include_wiki:
                target_collections.append(name)
            elif name.startswith("tag_") and include_rag:
                target_collections.append(name)

        self.collections_total = len(target_collections)
        stats["collections"] = len(target_collections)
        total_chunks = 0

        for col_name in target_collections:
            if self._cancel:
                return
            self.current_collection = col_name

            try:
                collection = chroma_client.get_collection(col_name)
            except Exception:
                self.collections_done += 1
                continue

            col_count = collection.count()
            if col_count == 0:
                self.collections_done += 1
                continue

            npz_path = vectors_dir / f"{col_name}.npz"
            await asyncio.to_thread(
                self._export_one_collection, collection, col_count, npz_path
            )
            total_chunks += col_count
            self.collections_done += 1

        stats["total_chunks"] = total_chunks

    def _export_one_collection(self, collection, col_count: int, npz_path: Path):
        """Export a single ChromaDB collection to .npz (runs in thread)."""
        all_ids = []
        all_embeddings = []
        all_documents = []
        all_metadatas = []

        for offset in range(0, col_count, CHROMA_BATCH_SIZE):
            if self._cancel:
                return
            batch = collection.get(
                include=["embeddings", "documents", "metadatas"],
                limit=CHROMA_BATCH_SIZE,
                offset=offset,
            )
            all_ids.extend(batch["ids"])
            all_embeddings.extend(batch["embeddings"])
            all_documents.extend(batch["documents"] or [])
            all_metadatas.extend(
                json.dumps(m) if m else "{}" for m in (batch["metadatas"] or [])
            )
            self.chunks_exported += len(batch["ids"])

        np.savez_compressed(
            str(npz_path),
            ids=np.array(all_ids, dtype=object),
            embeddings=np.array(all_embeddings, dtype=np.float32),
            documents=np.array(all_documents, dtype=object),
            metadatas=np.array(all_metadatas, dtype=object),
        )

    def _build_zip(self, tmp_dir: Path, out_path: Path):
        """Package temp directory into a ZIP file."""
        with zipfile.ZipFile(out_path, "w") as zf:
            for file in sorted(tmp_dir.rglob("*")):
                if file.is_file():
                    arcname = file.relative_to(tmp_dir).as_posix()
                    # .npz files are already compressed internally
                    compress = zipfile.ZIP_STORED if file.suffix == ".npz" \
                        else zipfile.ZIP_DEFLATED
                    zf.write(file, arcname, compress_type=compress)

    @property
    def output_path(self) -> Optional[Path]:
        return self._output_path


# ── Importer ─────────────────────────────────────────────────────────────────

class BackupImporter:
    def __init__(self):
        self.is_running = False
        self.phase = "idle"
        self.current_collection = ""
        self.collections_done = 0
        self.collections_total = 0
        self.chunks_imported = 0
        self.error = None
        self.warnings: list[str] = []
        self._cancel = False

    def get_status(self) -> dict:
        return {
            "is_running": self.is_running,
            "phase": self.phase,
            "current_collection": self.current_collection,
            "collections_done": self.collections_done,
            "collections_total": self.collections_total,
            "chunks_imported": self.chunks_imported,
            "error": self.error,
            "warnings": list(self.warnings),
        }

    def stop(self):
        self._cancel = True

    @staticmethod
    def validate_archive(file_path: Path) -> dict:
        """Validate a .georag archive without importing. Returns manifest + warnings."""
        warnings = []
        try:
            with zipfile.ZipFile(file_path, "r") as zf:
                if "manifest.json" not in zf.namelist():
                    return {"valid": False, "error": "Missing manifest.json"}
                manifest = json.loads(zf.read("manifest.json"))

                if manifest.get("format_version", 0) > FORMAT_VERSION:
                    return {"valid": False,
                            "error": f"Unsupported format version {manifest['format_version']}"}

                # Check embedding model
                src_model = manifest.get("embedding_model", "")
                src_dim = manifest.get("embedding_dim", 0)
                if src_dim != EMBEDDING_DIM:
                    return {"valid": False,
                            "error": f"Embedding dimension mismatch: archive has {src_dim}, "
                                     f"this system uses {EMBEDDING_DIM}. "
                                     "Vectors cannot be imported."}
                if src_model != EMBEDDING_MODEL:
                    warnings.append(
                        f"Embedding model differs: archive used '{src_model}', "
                        f"this system uses '{EMBEDDING_MODEL}'. "
                        "Vectors will be imported but search quality may differ."
                    )

                return {
                    "valid": True,
                    "manifest": manifest,
                    "warnings": warnings,
                    "size_bytes": file_path.stat().st_size,
                }
        except zipfile.BadZipFile:
            return {"valid": False, "error": "File is not a valid ZIP archive"}
        except Exception as e:
            return {"valid": False, "error": str(e)}

    async def run(self, file_path: Path):
        if self.is_running:
            return
        self.is_running = True
        self.phase = "idle"
        self.current_collection = ""
        self.collections_done = 0
        self.collections_total = 0
        self.chunks_imported = 0
        self.error = None
        self.warnings = []
        self._cancel = False

        try:
            # ── Phase: validating ──
            self.phase = "validating"
            await asyncio.sleep(0)

            validation = self.validate_archive(file_path)
            if not validation["valid"]:
                self.error = validation["error"]
                self.phase = "error"
                return
            manifest = validation["manifest"]
            self.warnings = validation.get("warnings", [])
            includes = manifest.get("includes", {})

            # ── Phase: clearing ──
            self.phase = "clearing"
            await asyncio.sleep(0)

            # Safety backup of metadata.db
            backup_db = METADATA_DB.with_suffix(".db.pre_import")
            if METADATA_DB.exists():
                shutil.copy2(METADATA_DB, backup_db)
                logger.info("Safety backup: %s", backup_db)

            # Clear ChromaDB collections
            try:
                collections = chroma_client.list_collections()
                for col in collections:
                    name = col if isinstance(col, str) else col.name
                    try:
                        chroma_client.delete_collection(name)
                    except Exception:
                        pass
            except Exception as e:
                logger.warning("Error clearing ChromaDB: %s", e)

            # Clear SQLite tables in FK dependency order
            db = SessionLocal()
            try:
                db.query(Message).delete()
                db.query(Conversation).delete()
                db.query(FileTag).delete()
                db.query(File).delete()
                db.query(Tag).delete()
                if includes.get("wiki", False):
                    db.query(WikiLog).delete()
                    db.query(WikiPage).delete()
                db.commit()
            except Exception as e:
                db.rollback()
                raise RuntimeError(f"Failed to clear database: {e}")
            finally:
                db.close()

            if self._cancel:
                return

            # ── Phase: sqlite ──
            self.phase = "sqlite"
            await asyncio.sleep(0)

            with zipfile.ZipFile(file_path, "r") as zf:
                db = SessionLocal()
                try:
                    await self._import_sqlite(db, zf, includes)
                    db.commit()
                except Exception as e:
                    db.rollback()
                    raise RuntimeError(f"SQLite import failed: {e}")
                finally:
                    db.close()

            if self._cancel:
                return

            # ── Phase: vectors ──
            self.phase = "vectors"
            await asyncio.sleep(0)

            with zipfile.ZipFile(file_path, "r") as zf:
                await self._import_vectors(zf)

            if self._cancel:
                return

            # ── Phase: finalizing ──
            self.phase = "finalizing"
            await asyncio.sleep(0)

            db = SessionLocal()
            try:
                # Recompute tag file_counts
                for tag in db.query(Tag).all():
                    tag.file_count = db.query(FileTag).filter(
                        FileTag.tag_id == tag.id
                    ).count()
                db.commit()

                # Rebuild wiki index and backlinks if wiki was imported
                if includes.get("wiki", False):
                    from backend.services.wiki_service import (
                        compute_backlinks, build_index_page,
                    )
                    compute_backlinks(db)
                    build_index_page(db)

            finally:
                db.close()

            self.phase = "done"
            logger.info("Backup import complete: %d chunks imported", self.chunks_imported)

        except Exception as e:
            logger.error("Backup import failed: %s", e)
            self.error = str(e)
            self.phase = "error"
        finally:
            self.is_running = False
            # Clean up uploaded file
            if file_path.exists():
                try:
                    file_path.unlink()
                except Exception:
                    pass

    async def _import_sqlite(self, db: Session, zf: zipfile.ZipFile,
                             includes: dict):
        """Import SQLite data from JSONL files in the archive."""
        # Tags first (referenced by file_tags)
        if "tags.json" in zf.namelist():
            tags = json.loads(zf.read("tags.json"))
            for t in tags:
                db.add(Tag(
                    id=t["id"], name=t["name"],
                    display_name=t["display_name"],
                    description=t.get("description"),
                    color=t.get("color", "#6c757d"),
                    file_count=t.get("file_count", 0),
                ))
            db.flush()

        # Files
        if "files.jsonl" in zf.namelist():
            for line in zf.read("files.jsonl").decode("utf-8").splitlines():
                if not line.strip():
                    continue
                d = json.loads(line)
                db.add(File(
                    id=d["id"],
                    relative_path=d["relative_path"],
                    filename=d["filename"],
                    extension=d.get("extension"),
                    size_bytes=d.get("size_bytes"),
                    modified_time=d.get("modified_time"),
                    parent_directory=d.get("parent_directory"),
                    content_hash=d.get("content_hash"),
                    scan_status=d.get("scan_status", "new"),
                    extracted_text_preview=d.get("extracted_text_preview"),
                    chunk_count=d.get("chunk_count", 0),
                    processed_at=_parse_datetime(d.get("processed_at")),
                    auto_tagged=d.get("auto_tagged", 0),
                    auto_tag_confidence=d.get("auto_tag_confidence"),
                ))
            db.flush()

        # FileTags
        if "file_tags.jsonl" in zf.namelist():
            for line in zf.read("file_tags.jsonl").decode("utf-8").splitlines():
                if not line.strip():
                    continue
                d = json.loads(line)
                db.add(FileTag(
                    id=d["id"],
                    file_id=d["file_id"],
                    tag_id=d["tag_id"],
                    source=d.get("source", "manual"),
                    confidence=d.get("confidence"),
                    created_at=_parse_datetime(d.get("created_at")),
                ))
            db.flush()

        # Wiki pages
        if includes.get("wiki", False) and "wiki_pages.jsonl" in zf.namelist():
            for line in zf.read("wiki_pages.jsonl").decode("utf-8").splitlines():
                if not line.strip():
                    continue
                d = json.loads(line)
                db.add(WikiPage(
                    id=d["id"],
                    slug=d["slug"],
                    title=d["title"],
                    content=d.get("content", ""),
                    category=d.get("category", "general"),
                    summary=d.get("summary"),
                    source_files=d.get("source_files", []),
                    backlinks=d.get("backlinks", []),
                    created_at=_parse_datetime(d.get("created_at")),
                    updated_at=_parse_datetime(d.get("updated_at")),
                ))
            db.flush()

        # Wiki logs
        if includes.get("wiki", False) and "wiki_log.jsonl" in zf.namelist():
            for line in zf.read("wiki_log.jsonl").decode("utf-8").splitlines():
                if not line.strip():
                    continue
                d = json.loads(line)
                db.add(WikiLog(
                    id=d["id"],
                    operation=d["operation"],
                    detail=d.get("detail"),
                    pages_affected=d.get("pages_affected", []),
                    created_at=_parse_datetime(d.get("created_at")),
                ))
            db.flush()

    async def _import_vectors(self, zf: zipfile.ZipFile):
        """Import ChromaDB collections from .npz files."""
        vector_files = [n for n in zf.namelist()
                        if n.startswith("vectors/") and n.endswith(".npz")]
        self.collections_total = len(vector_files)

        for vf_name in vector_files:
            if self._cancel:
                return
            # Extract collection name: "vectors/tag_piling.npz" -> "tag_piling"
            col_name = Path(vf_name).stem
            self.current_collection = col_name

            npz_bytes = zf.read(vf_name)
            await asyncio.to_thread(self._import_one_collection, col_name, npz_bytes)
            self.collections_done += 1

    def _import_one_collection(self, col_name: str, npz_bytes: bytes):
        """Import a single .npz into a ChromaDB collection (runs in thread)."""
        data = np.load(io.BytesIO(npz_bytes), allow_pickle=True)

        ids = data["ids"].tolist()
        embeddings = data["embeddings"].tolist()
        documents = data["documents"].tolist()
        metadatas_raw = data["metadatas"].tolist()

        # Parse metadata JSON strings back to dicts
        metadatas = []
        for m in metadatas_raw:
            try:
                metadatas.append(json.loads(m) if m else {})
            except (json.JSONDecodeError, TypeError):
                metadatas.append({})

        if not ids:
            return

        # Determine if this is a tag collection or the wiki collection
        if col_name == WIKI_COLLECTION_NAME:
            collection = chroma_client.get_or_create_collection(
                name=WIKI_COLLECTION_NAME,
                metadata={"hnsw:space": "cosine"},
            )
        elif col_name.startswith("tag_"):
            tag_name = col_name[4:]  # strip "tag_" prefix
            collection = get_or_create_collection(tag_name)
        else:
            # Unknown collection type, create as-is
            collection = chroma_client.get_or_create_collection(
                name=col_name,
                metadata={"hnsw:space": "cosine"},
            )

        # Batch upsert (same batch size as vector_store.add_chunks)
        batch_size = 5000
        for i in range(0, len(ids), batch_size):
            collection.upsert(
                ids=ids[i:i + batch_size],
                embeddings=embeddings[i:i + batch_size],
                documents=documents[i:i + batch_size],
                metadatas=metadatas[i:i + batch_size],
            )
            self.chunks_imported += (min(i + batch_size, len(ids)) - i)


# ── Module-level singletons ──────────────────────────────────────────────────

exporter = BackupExporter()
importer = BackupImporter()
