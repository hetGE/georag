"""Orchestrates extract -> chunk -> embed -> store pipeline."""
import asyncio
import traceback
from pathlib import Path

from backend.config import ENGINEERING_ROOT
from backend.models.database import SessionLocal
from backend.models.schemas import File, Tag, FileTag
from backend.services.chunker import chunk_text
from backend.services.embedding_client import embed_batch
from backend.services.vector_store import add_chunks
from backend.services.tagger import tag_by_folder, tag_by_llm
from backend.services.extractors.pdf_extractor import extract_pdf
from backend.services.extractors.docx_extractor import extract_docx
from backend.services.extractors.xlsx_extractor import extract_xlsx
from backend.services.extractors.pptx_extractor import extract_pptx
from backend.services.extractors.image_extractor import extract_image
from backend.services.extractors.text_extractor import extract_text


EXTRACTOR_MAP = {
    "pdf": extract_pdf,
    "docx": extract_docx,
    "doc": None,  # Skip .doc for now (requires antiword/LibreOffice)
    "xlsx": extract_xlsx,
    "xls": None,  # Skip .xls for now
    "xlsm": extract_xlsx,
    "pptx": extract_pptx,
    "ppt": None,  # Skip .ppt for now
    "png": extract_image,
    "jpg": extract_image,
    "jpeg": extract_image,
    "gif": extract_image,
    "bmp": extract_image,
    "tiff": extract_image,
    "txt": extract_text,
    "html": extract_text,
    "htm": extract_text,
    "rtf": extract_text,
    "csv": extract_text,
    "md": extract_text,
    "dwg": None,  # Filename-only indexing
    "dxf": None,
}


class DocumentProcessor:
    def __init__(self):
        self.is_running = False
        self.total_files = 0
        self.processed_files = 0
        self.failed_files = 0
        self.current_file = None
        self.errors = []
        self._stop_flag = False

    def stop(self):
        self._stop_flag = True

    async def run(self, tag_names: list[str] = None, file_ids: list[int] = None, reprocess: bool = False):
        """Run the processing pipeline."""
        self.is_running = True
        self._stop_flag = False
        self.processed_files = 0
        self.failed_files = 0
        self.errors = []
        self.current_file = None

        db = SessionLocal()
        try:
            # Get files to process
            query = db.query(File)
            if file_ids:
                query = query.filter(File.id.in_(file_ids))
            if not reprocess:
                query = query.filter(File.scan_status.in_(["new", "failed"]))

            files = query.all()
            self.total_files = len(files)

            for file_record in files:
                if self._stop_flag:
                    break

                self.current_file = file_record.filename
                try:
                    await self._process_file(file_record, db, tag_names)
                    self.processed_files += 1
                except Exception as e:
                    self.failed_files += 1
                    self.errors.append(f"{file_record.filename}: {str(e)}")
                    file_record.scan_status = "failed"
                    db.commit()

        except Exception as e:
            self.errors.append(f"Pipeline error: {str(e)}")
        finally:
            self.is_running = False
            self.current_file = None
            db.close()

    async def _process_file(self, file_record: File, db, tag_names: list[str] = None):
        """Process a single file: extract, chunk, embed, store."""
        ext = file_record.extension
        extractor = EXTRACTOR_MAP.get(ext)

        full_path = ENGINEERING_ROOT / file_record.relative_path

        if not full_path.exists():
            file_record.scan_status = "skipped"
            db.commit()
            return

        # Extract text
        if extractor is None:
            # Filename-only indexing for unsupported formats
            text = f"File: {file_record.filename}\nPath: {file_record.relative_path}"
            file_record.scan_status = "processed"
        else:
            text = await extractor(str(full_path))
            if not text or not text.strip():
                file_record.scan_status = "skipped"
                db.commit()
                return

        file_record.extracted_text_preview = text[:500]

        # Auto-tag if no specific tags provided
        file_tags = []
        if tag_names:
            file_tags = [(t, 1.0) for t in tag_names]
        else:
            # Folder heuristic
            file_tags = tag_by_folder(file_record.relative_path)

            # LLM tagging if no folder hints found
            if not file_tags and text:
                try:
                    file_tags = await tag_by_llm(text[:2000], file_record.filename)
                except Exception:
                    pass

        # Save tags to database
        for tag_name, confidence in file_tags:
            tag = db.query(Tag).filter(Tag.name == tag_name).first()
            if not tag:
                continue
            existing = db.query(FileTag).filter(
                FileTag.file_id == file_record.id,
                FileTag.tag_id == tag.id
            ).first()
            if not existing:
                source = "folder_hint" if confidence == 0.7 else "auto"
                db.add(FileTag(
                    file_id=file_record.id, tag_id=tag.id,
                    source=source, confidence=confidence,
                ))
                tag.file_count = db.query(FileTag).filter(FileTag.tag_id == tag.id).count() + 1

        if file_tags:
            file_record.auto_tagged = 1
            file_record.auto_tag_confidence = max(c for _, c in file_tags)

        # Chunk and embed
        metadata = {
            "file_path": file_record.relative_path,
            "filename": file_record.filename,
            "file_type": ext,
            "parent_dir": file_record.parent_directory,
        }
        chunks = chunk_text(text, metadata)
        file_record.chunk_count = len(chunks)

        if chunks and file_tags:
            # Embed all chunks
            chunk_texts = [c["text"] for c in chunks]
            try:
                embeddings = await embed_batch(chunk_texts)
            except Exception as e:
                # Store without embeddings if embedding fails
                file_record.scan_status = "processed"
                db.commit()
                raise Exception(f"Embedding failed: {e}")

            # Store in each tag's collection
            for tag_name, _ in file_tags:
                ids = [f"{file_record.relative_path}::chunk_{c['metadata']['chunk_index']}" for c in chunks]
                metadatas = [c["metadata"] for c in chunks]
                add_chunks(tag_name, ids, embeddings, chunk_texts, metadatas)

        file_record.scan_status = "processed"
        db.commit()
