"""File system discovery - walks Engineering/ and catalogs files in SQLite."""
import os
from pathlib import Path

from backend.config import (
    ENGINEERING_ROOT, SKIP_DIRS, SUPPORTED_EXTENSIONS,
    JUNK_FILENAMES, JUNK_FILENAME_PREFIXES,
)
from backend.models.database import SessionLocal
from backend.models.schemas import File, FileTag, Tag
from backend.services.vector_store import delete_file_from_tag


def scan_engineering_directory() -> dict:
    """Walk the Engineering directory and insert/update files in the database.
    Returns dict with files_found and files_removed counts."""
    db = SessionLocal()
    try:
        # Get existing paths for fast lookup
        existing_paths = set(
            row[0] for row in db.query(File.relative_path).all()
        )

        new_files = []
        found_paths = set()
        count = 0
        new_count = 0

        for root, dirs, files in os.walk(ENGINEERING_ROOT):
            # Skip excluded directories
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]

            for filename in files:
                # Skip junk/system files
                if filename in JUNK_FILENAMES or any(
                    filename.startswith(p) for p in JUNK_FILENAME_PREFIXES
                ):
                    continue

                full_path = Path(root) / filename
                ext = full_path.suffix.lower().lstrip(".")

                if ext not in SUPPORTED_EXTENSIONS:
                    continue

                try:
                    relative_path = str(full_path.relative_to(ENGINEERING_ROOT))
                except ValueError:
                    continue

                found_paths.add(relative_path)

                if relative_path in existing_paths:
                    count += 1
                    continue

                try:
                    stat = full_path.stat()
                    size_bytes = stat.st_size
                    modified_time = stat.st_mtime
                except OSError:
                    size_bytes = 0
                    modified_time = 0

                parent_dir = str(Path(relative_path).parent)

                new_files.append(File(
                    relative_path=relative_path,
                    filename=filename,
                    extension=ext,
                    size_bytes=size_bytes,
                    modified_time=modified_time,
                    parent_directory=parent_dir,
                    scan_status="new",
                ))
                count += 1
                new_count += 1

                # Batch insert every 500 files
                if len(new_files) >= 500:
                    db.bulk_save_objects(new_files)
                    db.commit()
                    new_files.clear()

        # Insert remaining
        if new_files:
            db.bulk_save_objects(new_files)
            db.commit()

        # Remove files that no longer exist on disk
        deleted_paths = existing_paths - found_paths
        removed = len(deleted_paths)
        if deleted_paths:
            # Clean up vector store chunks for deleted files
            stale_files = db.query(File).filter(File.relative_path.in_(deleted_paths)).all()
            for f in stale_files:
                for ft in f.tags:
                    tag = db.get(Tag, ft.tag_id)
                    if tag:
                        delete_file_from_tag(tag.name, f.relative_path)
                        tag.file_count = max(0, tag.file_count - 1)
                db.delete(f)
            db.commit()

        # Clean up junk files that were added in previous versions
        from sqlalchemy import or_
        junk_filters = [File.filename.in_(JUNK_FILENAMES)] + [
            File.filename.like(f"{p}%") for p in JUNK_FILENAME_PREFIXES
        ]
        junk_files = db.query(File).filter(or_(*junk_filters)).all()
        cleaned = len(junk_files)
        if junk_files:
            for f in junk_files:
                for ft in f.tags:
                    tag = db.get(Tag, ft.tag_id)
                    if tag:
                        delete_file_from_tag(tag.name, f.relative_path)
                        tag.file_count = max(0, tag.file_count - 1)
                db.delete(f)
            db.commit()

        return {
            "files_found": count,
            "files_new": new_count,
            "files_removed": removed,
            "files_cleaned": cleaned,
        }

    finally:
        db.close()
