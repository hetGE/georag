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
                        # Verify the OCR'd PDF is now actually extractable before
                        # resetting to "new" — without this check, files that already
                        # had an OCR layer (PriorOcrFoundError) or whose OCR output
                        # pdfplumber still can't read would loop forever.
                        from backend.services.extractors.pdf_extractor import extract_pdf
                        try:
                            extracted = await asyncio.to_thread(extract_pdf, str(abs_path))
                        except Exception:
                            extracted = ""

                        if extracted and extracted.strip():
                            file_rec.scan_status = "new"
                            file_rec.extracted_text_preview = None
                            file_rec.chunk_count = None
                            db.commit()
                            self.ocr_success += 1
                            logger.info("OCR success + text verified: %s", file_rec.filename)
                        else:
                            # All pages were skipped (existing text layer) but
                            # pdfplumber still can't read anything — the text
                            # layer is fake/corrupt.  Rasterise every page and
                            # re-OCR from scratch.
                            logger.info(
                                "No extractable text after skip_text OCR, "
                                "retrying with force_ocr: %s",
                                file_rec.filename,
                            )
                            success2 = await asyncio.to_thread(
                                self._ocr_single_file, abs_path, force_ocr=True
                            )
                            if success2:
                                try:
                                    extracted2 = await asyncio.to_thread(
                                        extract_pdf, str(abs_path)
                                    )
                                except Exception:
                                    extracted2 = ""
                            else:
                                extracted2 = ""

                            if extracted2 and extracted2.strip():
                                file_rec.scan_status = "new"
                                file_rec.extracted_text_preview = None
                                file_rec.chunk_count = None
                                db.commit()
                                self.ocr_success += 1
                                logger.info(
                                    "OCR force success + text verified: %s",
                                    file_rec.filename,
                                )
                            else:
                                # Even force OCR couldn't produce readable text
                                file_rec.scan_status = "ocr_failed"
                                db.commit()
                                self.ocr_failed += 1
                                msg = f"{file_rec.filename}: OCR applied but still no extractable text"
                                self.errors.append(msg)
                                logger.warning("OCR: %s", msg)
                    else:
                        self.ocr_failed += 1
                except Exception as e:
                    self.ocr_failed += 1
                    detail = str(e) or type(e).__name__
                    err_msg = f"{file_rec.filename}: {detail}"
                    self.errors.append(err_msg)
                    file_rec.scan_status = "ocr_failed"
                    db.commit()
                    logger.warning("OCR failed: %s", err_msg, exc_info=True)

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

    def _repair_pdf(self, src_path) -> str:
        """Re-encode a PDF with pymupdf to fix corrupted content streams.

        Returns the path to the repaired temp file.  Caller must delete it.
        """
        import fitz  # pymupdf

        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            repaired_path = f.name

        doc = fitz.open(str(src_path))
        # garbage=4  → merge duplicate streams
        # deflate=True → re-compress all streams (fixes corrupt zlib data)
        # clean=True  → sanitise content streams
        doc.save(repaired_path, garbage=4, deflate=True, clean=True)
        doc.close()
        logger.info("Repaired PDF %s → %s", src_path.name, repaired_path)
        return repaired_path

    def _ocr_single_file(self, abs_path, force_ocr=False):
        """Synchronous OCR of a single PDF. Returns True on success.

        force_ocr=True rasterises every page and re-OCRs from scratch,
        bypassing any existing (possibly fake/corrupt) text layer.
        """
        import os
        import ocrmypdf

        # OCR to a temp file, then move over the original (atomic-ish)
        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as tmp:
            tmp_path = tmp.name

        repaired_path = None

        def _run_ocr(input_path, **extra):
            kwargs = dict(
                language="+".join(OCR_LANGUAGES),
                optimize=0,       # disable image optimisation (requires ghostscript)
                progress_bar=False,
            )
            if force_ocr:
                kwargs["force_ocr"] = True   # rasterise all pages, ignore existing text
            else:
                kwargs["skip_text"] = True   # handle mixed PDFs (some pages already have text)
            kwargs.update(extra)
            ocrmypdf.ocr(input_path, tmp_path, **kwargs)

        try:
            try:
                _run_ocr(abs_path)
            except ocrmypdf.exceptions.DigitalSignatureError:
                # PDF is digitally signed — ocrmypdf refuses to modify it by default.
                # invalidate_digital_signatures=True lets it proceed; the existing
                # signature becomes invalid (which is acceptable here).
                logger.info(
                    "Digital signature detected in %s — retrying with signature invalidation",
                    abs_path.name,
                )
                _run_ocr(abs_path, invalidate_digital_signatures=True)
            except ocrmypdf.exceptions.InputFileError:
                # PDF has corrupted internal structure (e.g. bad content stream).
                # Re-encode with pymupdf to normalise streams, then retry.
                logger.info(
                    "Corrupt PDF detected in %s — attempting repair with pymupdf",
                    abs_path.name,
                )
                repaired_path = self._repair_pdf(abs_path)
                _run_ocr(repaired_path)

            shutil.move(tmp_path, abs_path)
            return True
        except ocrmypdf.exceptions.PriorOcrFoundError:
            # Already has OCR — treat as success, just reset status
            try:
                os.unlink(tmp_path)
            except OSError:
                pass
            return True
        except Exception:
            try:
                os.unlink(tmp_path)
            except OSError:
                pass
            raise
        finally:
            if repaired_path:
                try:
                    os.unlink(repaired_path)
                except OSError:
                    pass
