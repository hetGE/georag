"""OCR scanned/image-based PDFs so they become searchable."""
import asyncio
import logging
import shutil
import tempfile
import time

from backend.config import ENGINEERING_ROOT, OCR_LANGUAGES
from backend.models.database import SessionLocal
from backend.models.schemas import File

logger = logging.getLogger(__name__)


class OCRProcessor:
    def __init__(self):
        self.is_running = False
        self.total_files = 0
        self.processed_files = 0
        self.ocr_success = 0
        self.ocr_failed = 0
        self.current_file = None
        self._stop_flag = False
        self.errors = []

    def stop(self):
        self._stop_flag = True

    async def run(self):
        """OCR all failed PDF files, overwrite originals, reset to new."""
        self.is_running = True
        self._stop_flag = False
        self.processed_files = 0
        self.ocr_success = 0
        self.ocr_failed = 0
        self.current_file = None
        self.errors = []
        start = time.time()
        logger.info("OCR processing started")

        db = SessionLocal()
        try:
            failed_pdfs = (
                db.query(File)
                .filter(File.scan_status == "failed", File.extension == "pdf")
                .all()
            )

            if not failed_pdfs:
                logger.info("OCR: no failed PDFs found")
                self.errors.append("No failed PDF files found.")
                return

            self.total_files = len(failed_pdfs)
            logger.info("OCR: %d failed PDFs to process", self.total_files)

            for file_rec in failed_pdfs:
                if self._stop_flag:
                    logger.info("OCR: stopped by user after %d files", self.processed_files)
                    break

                self.current_file = file_rec.relative_path or file_rec.filename
                abs_path = ENGINEERING_ROOT / file_rec.relative_path

                if not abs_path.exists():
                    self.ocr_failed += 1
                    self.processed_files += 1
                    self.errors.append(f"{file_rec.filename}: file not found")
                    logger.warning("OCR: file not found: %s", abs_path)
                    continue

                try:
                    success = await asyncio.to_thread(
                        self._ocr_single_file, abs_path
                    )
                    if success:
                        # Reset file so normal pipeline will re-process it
                        file_rec.scan_status = "new"
                        file_rec.extracted_text_preview = None
                        file_rec.chunk_count = None
                        db.commit()
                        self.ocr_success += 1
                        logger.info("OCR success: %s", file_rec.filename)
                    else:
                        self.ocr_failed += 1
                except Exception as e:
                    self.ocr_failed += 1
                    err_msg = f"{file_rec.filename}: {e}"
                    self.errors.append(err_msg)
                    logger.warning("OCR failed: %s", err_msg)

                self.processed_files += 1

        except Exception as e:
            logger.error("OCR processor error: %s", e, exc_info=True)
            self.errors.append(f"OCR processor error: {e}")
        finally:
            elapsed = time.time() - start
            logger.info(
                "OCR finished: %d success, %d failed in %.1fs",
                self.ocr_success, self.ocr_failed, elapsed,
            )
            self.current_file = None
            self.is_running = False
            db.close()

    def _ocr_single_file(self, abs_path):
        """Synchronous OCR of a single PDF. Returns True on success."""
        import ocrmypdf

        # OCR to a temp file, then move over the original (atomic-ish)
        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            ocrmypdf.ocr(
                abs_path,
                tmp_path,
                language="+".join(OCR_LANGUAGES),
                skip_text=True,      # handle mixed PDFs (some pages already have text)
                optimize=1,
                progress_bar=False,
            )
            shutil.move(tmp_path, abs_path)
            return True
        except ocrmypdf.exceptions.PriorOcrFoundError:
            # Already has OCR — treat as success, just reset status
            try:
                import os
                os.unlink(tmp_path)
            except OSError:
                pass
            return True
        except Exception:
            try:
                import os
                os.unlink(tmp_path)
            except OSError:
                pass
            raise
