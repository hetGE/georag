"""Orchestrates extract -> chunk -> embed -> store pipeline with concurrency."""
import asyncio
import traceback
from pathlib import Path

from backend.config import ENGINEERING_ROOT
from backend.models.database import SessionLocal
from backend.models.schemas import File, Tag, FileTag
from backend.services.chunker import chunk_text
from backend.services.embedding_client import embed_batch
from backend.services.vector_store import add_chunks
from backend.services.tagger import tag_by_heuristic, tag_by_llm, tag_batch_by_llm
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

# Concurrency controls
_llm_semaphore = asyncio.Semaphore(1)  # LM Studio serves one model request at a time
_db_lock = asyncio.Lock()

BATCH_SIZE = 10  # Files processed concurrently & LLM batch size


class DocumentProcessor:
    def __init__(self):
        self.is_running = False
        self.total_files = 0
        self.processed_files = 0
        self.failed_files = 0
        self.current_file = None
        self.errors = []
        self._stop_flag = False
        self.new_tags_added = 0
        self.files_newly_tagged = 0

    def stop(self):
        self._stop_flag = True

    async def run(self, tag_names: list[str] = None, file_ids: list[int] = None, reprocess: bool = False):
        """Run the processing pipeline with concurrent file processing."""
        self.is_running = True
        self._stop_flag = False
        self.processed_files = 0
        self.failed_files = 0
        self.errors = []
        self.current_file = None
        self.new_tags_added = 0
        self.files_newly_tagged = 0

        db = SessionLocal()
        try:
            # Get files to process
            query = db.query(File)
            if file_ids:
                query = query.filter(File.id.in_(file_ids))
            if not reprocess:
                query = query.filter(File.scan_status == "new")

            files = query.all()
            self.total_files = len(files)

            # Reset statuses so progress starts from 0%
            if reprocess and files:
                for f in files:
                    f.scan_status = "new"
                db.commit()

            # Process files in batches of BATCH_SIZE
            for batch_start in range(0, len(files), BATCH_SIZE):
                if self._stop_flag:
                    break

                batch = files[batch_start:batch_start + BATCH_SIZE]

                # Phase 1: Extract text concurrently for the batch
                extraction_results = await asyncio.gather(
                    *[self._extract_file(f) for f in batch],
                    return_exceptions=True,
                )

                if self._stop_flag:
                    break

                # Collect files that need LLM tagging
                llm_tagging_queue = []  # (file_record, text, index_in_batch)
                file_data = []  # (file_record, text, file_tags_or_None)

                for i, (file_record, result) in enumerate(zip(batch, extraction_results)):
                    if isinstance(result, Exception):
                        self.failed_files += 1
                        self.errors.append(f"{file_record.filename}: {result}")
                        async with _db_lock:
                            file_record.scan_status = "failed"
                            db.commit()
                        continue

                    text, was_skipped = result
                    if was_skipped:
                        self.failed_files += 1
                        self.errors.append(f"{file_record.filename}: no extractable content")
                        async with _db_lock:
                            file_record.scan_status = "failed"
                            db.commit()
                        self.processed_files += 1
                        continue

                    # Determine tags
                    if tag_names:
                        file_tags = [(t, 1.0) for t in tag_names]
                        file_data.append((file_record, text, file_tags))
                    else:
                        heuristic_tags = tag_by_heuristic(file_record.relative_path, file_record.filename)

                        if text:
                            # Always queue for LLM so new/additional tags are considered
                            llm_tagging_queue.append((file_record, text))
                            file_data.append((file_record, text, heuristic_tags))
                        elif heuristic_tags:
                            file_data.append((file_record, text, heuristic_tags))
                        else:
                            file_data.append((file_record, text, []))

                # Phase 2: Batch LLM tagging for files that need it
                if self._stop_flag:
                    break

                if llm_tagging_queue:
                    async with _llm_semaphore:
                        try:
                            files_info = [
                                {"filename": fr.filename, "text_preview": txt[:2000]}
                                for fr, txt in llm_tagging_queue
                            ]
                            llm_results = await tag_batch_by_llm(files_info)

                            # Merge LLM results with any existing heuristic tags
                            for fr, txt in llm_tagging_queue:
                                llm_tags = llm_results.get(fr.filename, [])
                                for j, (fd_rec, fd_txt, fd_tags) in enumerate(file_data):
                                    if fd_rec.id == fr.id:
                                        existing_names = {t for t, _ in (fd_tags or [])}
                                        merged = list(fd_tags or []) + [
                                            (t, c) for t, c in llm_tags if t not in existing_names
                                        ]
                                        file_data[j] = (fd_rec, fd_txt, merged)
                                        break
                        except Exception:
                            # Fallback: try individual LLM tagging
                            for fr, txt in llm_tagging_queue:
                                try:
                                    async with _llm_semaphore:
                                        llm_tags = await tag_by_llm(txt[:2000], fr.filename)
                                except Exception:
                                    llm_tags = []
                                for j, (fd_rec, fd_txt, fd_tags) in enumerate(file_data):
                                    if fd_rec.id == fr.id:
                                        existing_names = {t for t, _ in (fd_tags or [])}
                                        merged = list(fd_tags or []) + [
                                            (t, c) for t, c in llm_tags if t not in existing_names
                                        ]
                                        file_data[j] = (fd_rec, fd_txt, merged)
                                        break

                if self._stop_flag:
                    break

                # Phase 3: Chunk, embed, store concurrently
                store_tasks = []
                for file_record, text, file_tags in file_data:
                    if file_tags is None:
                        file_tags = []
                    store_tasks.append(
                        self._chunk_embed_store(file_record, text, file_tags, db)
                    )

                results = await asyncio.gather(*store_tasks, return_exceptions=True)
                for file_record_data, result in zip(file_data, results):
                    file_record = file_record_data[0]
                    if isinstance(result, Exception):
                        self.failed_files += 1
                        self.errors.append(f"{file_record.filename}: {result}")
                        async with _db_lock:
                            file_record.scan_status = "failed"
                            db.commit()
                    else:
                        self.processed_files += 1

        except Exception as e:
            self.errors.append(f"Pipeline error: {str(e)}")
        finally:
            self.is_running = False
            self.current_file = None
            db.close()

    async def _extract_file(self, file_record: File) -> tuple[str, bool]:
        """Extract text from a file. Returns (text, was_skipped).
        Runs CPU-bound extractors in a thread pool."""
        ext = file_record.extension
        extractor = EXTRACTOR_MAP.get(ext)

        full_path = ENGINEERING_ROOT / file_record.relative_path

        if not full_path.exists():
            return ("", True)

        if extractor is None:
            # Filename-only indexing for unsupported formats
            text = f"File: {file_record.filename}\nPath: {file_record.relative_path}"
            return (text, False)

        # Dispatch: async extractors (image) stay async, sync ones go to thread pool
        if asyncio.iscoroutinefunction(extractor):
            async with _llm_semaphore:
                text = await extractor(str(full_path))
        else:
            text = await asyncio.to_thread(extractor, str(full_path))

        if not text or not text.strip():
            return ("", True)

        return (text, False)

    async def _chunk_embed_store(self, file_record: File, text: str,
                                  file_tags: list[tuple[str, float]], db):
        """Chunk text, embed, save tags, store in vector DB."""
        self.current_file = file_record.filename
        file_record.extracted_text_preview = text[:500]

        # Save tags to database
        async with _db_lock:
            file_got_new_tag = False
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
                    self.new_tags_added += 1
                    file_got_new_tag = True

            if file_got_new_tag:
                self.files_newly_tagged += 1

            if file_tags:
                file_record.auto_tagged = 1
                file_record.auto_tag_confidence = max(c for _, c in file_tags)

        # Chunk and embed
        ext = file_record.extension
        metadata = {
            "file_path": file_record.relative_path,
            "filename": file_record.filename,
            "file_type": ext,
            "parent_dir": file_record.parent_directory,
        }
        chunks = chunk_text(text, metadata)
        file_record.chunk_count = len(chunks)

        if chunks and file_tags:
            chunk_texts = [c["text"] for c in chunks]
            try:
                embeddings = await embed_batch(chunk_texts)
            except Exception as e:
                async with _db_lock:
                    file_record.scan_status = "processed"
                    db.commit()
                raise Exception(f"Embedding failed: {e}")

            # Store in each tag's collection
            for tag_name, _ in file_tags:
                ids = [f"{file_record.relative_path}::chunk_{c['metadata']['chunk_index']}" for c in chunks]
                metadatas = [c["metadata"] for c in chunks]
                add_chunks(tag_name, ids, embeddings, chunk_texts, metadatas)

        async with _db_lock:
            file_record.scan_status = "processed"
            db.commit()
