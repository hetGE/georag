"""File system discovery - walks Engineering/ and catalogs files in SQLite."""
import os
from pathlib import Path

from backend.config import ENGINEERING_ROOT, SKIP_DIRS, SUPPORTED_EXTENSIONS
from backend.models.database import SessionLocal
from backend.models.schemas import File


def scan_engineering_directory() -> int:
    """Walk the Engineering directory and insert/update files in the database.
    Returns the number of files found."""
    db = SessionLocal()
    try:
        # Get existing paths for fast lookup
        existing_paths = set(
            row[0] for row in db.query(File.relative_path).all()
        )

        new_files = []
        found_paths = set()
        count = 0

        for root, dirs, files in os.walk(ENGINEERING_ROOT):
            # Skip excluded directories
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]

            for filename in files:
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

                # Batch insert every 500 files
                if len(new_files) >= 500:
                    db.bulk_save_objects(new_files)
                    db.commit()
                    new_files.clear()

        # Insert remaining
        if new_files:
            db.bulk_save_objects(new_files)
            db.commit()

        return count

    finally:
        db.close()
