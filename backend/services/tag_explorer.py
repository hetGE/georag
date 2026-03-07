"""Discover new tag categories from untagged processed files."""
import asyncio
import json
import logging
import re
import time

from backend.models.database import SessionLocal

logger = logging.getLogger(__name__)
from backend.models.schemas import File, Tag, FileTag
from backend.services.llm_client import chat_completion
from backend.services.tagger import tag_by_heuristic, tag_batch_by_llm

EXPLORE_BATCH_SIZE = 10
MIN_FILES_PER_TAG = 10

# Colors that avoid the 10 default tag colors
EXPLORE_COLORS = [
    "#c0392c", "#2980b9", "#f1c40f", "#d35400", "#7f8c8d",
    "#2c3e50", "#1e8449", "#6c3483", "#b03a2e", "#148f77",
    "#d4ac0d", "#a04000", "#5b2c6f", "#1a5276", "#196f3d",
    "#b7950b", "#784212", "#4a235a", "#154360", "#0e6655",
]

_llm_semaphore = asyncio.Semaphore(1)


class TagExplorer:
    def __init__(self):
        self.is_running = False
        self.total_files = 0
        self.processed_files = 0
        self.current_batch = 0
        self.total_batches = 0
        self._stop_flag = False
        self.errors = []
        self.candidates = []
        self._raw_suggestions = {}  # {tag_name: {display_name, description, files: set()}}
        self.existing_tags_assigned = 0
        self.existing_files_tagged = 0
        self._existing_files_set = set()

    def stop(self):
        self._stop_flag = True

    def has_results(self):
        return len(self.candidates) > 0

    async def run(self):
        """Discover new tag categories from untagged processed files."""
        self.is_running = True
        self._stop_flag = False
        self.processed_files = 0
        self.current_batch = 0
        self.errors = []
        self.candidates = []
        self._raw_suggestions = {}
        self.existing_tags_assigned = 0
        self.existing_files_tagged = 0
        self._existing_files_set = set()
        explore_start = time.time()
        logger.info("Tag exploration started")

        db = SessionLocal()
        try:
            # Step 1: Find untagged processed files
            tagged_ids = db.query(FileTag.file_id).distinct()
            untagged_files = (
                db.query(File)
                .filter(File.scan_status == "processed")
                .filter(~File.id.in_(tagged_ids))
                .all()
            )

            if not untagged_files:
                logger.info("Tag exploration: no untagged processed files found")
                self.errors.append("No untagged processed files found.")
                return

            self.total_files = len(untagged_files)
            logger.info("Tag exploration: %d untagged files to analyze", len(untagged_files))
            self.total_batches = (len(untagged_files) + EXPLORE_BATCH_SIZE - 1) // EXPLORE_BATCH_SIZE

            # Step 3: Fetch existing tags
            existing_tags = {
                t.name: {"display_name": t.display_name, "description": t.description or t.display_name}
                for t in db.query(Tag).all()
            }
            existing_tag_names = set(existing_tags.keys())

            # Build file lookup for tag application
            file_lookup = {f.filename: f for f in untagged_files}

            # Cache Tag objects to avoid repeated queries
            tag_cache = {t.name: t for t in db.query(Tag).all()}

            # Step 4: Process in batches
            for batch_start in range(0, len(untagged_files), EXPLORE_BATCH_SIZE):
                if self._stop_flag:
                    break

                batch = untagged_files[batch_start:batch_start + EXPLORE_BATCH_SIZE]
                self.current_batch += 1
                logger.info("Explore batch %d/%d (%d files)", self.current_batch, self.total_batches, len(batch))

                files_info = []
                for f in batch:
                    preview = (f.extracted_text_preview or "")[:1500]
                    files_info.append({
                        "filename": f.filename,
                        "text_preview": preview,
                        "file_id": f.id,
                        "relative_path": f.relative_path or "",
                    })

                try:
                    # --- Step 4a: Heuristic tagging (instant) ---
                    for f in batch:
                        heuristic_tags = tag_by_heuristic(
                            f.relative_path or "", f.filename
                        )
                        for tag_name, confidence in heuristic_tags:
                            self._apply_tag(
                                db, file_lookup.get(f.filename), tag_name,
                                tag_cache, source="folder_hint", confidence=confidence,
                            )

                    # --- Step 4b: LLM existing tag classification ---
                    llm_files = [
                        {"filename": info["filename"], "text_preview": info["text_preview"]}
                        for info in files_info
                    ]
                    async with _llm_semaphore:
                        llm_results = await tag_batch_by_llm(llm_files)

                    for filename, tags in llm_results.items():
                        file_rec = file_lookup.get(filename)
                        if not file_rec:
                            continue
                        for tag_name, confidence in tags:
                            self._apply_tag(
                                db, file_rec, tag_name,
                                tag_cache, source="auto", confidence=confidence,
                            )

                    # --- Step 4c: LLM new tag discovery ---
                    logger.info("Explore: LLM new tag discovery for batch %d", self.current_batch)
                    discovered_names = set(self._raw_suggestions.keys())
                    async with _llm_semaphore:
                        new_tags = await self._discover_new_tags_batch(
                            files_info, existing_tags, discovered_names
                        )

                    # Accumulate new tag suggestions
                    for tag_name, info in new_tags.items():
                        if not isinstance(info, dict):
                            continue
                        if tag_name in existing_tag_names:
                            continue
                        if tag_name not in self._raw_suggestions:
                            self._raw_suggestions[tag_name] = {
                                "display_name": info.get("display_name", tag_name.replace("_", " ").title()),
                                "description": info.get("description", ""),
                                "files": set(),
                            }
                        for fname in info.get("filenames", []):
                            self._raw_suggestions[tag_name]["files"].add(fname)

                except Exception as e:
                    logger.warning("Explore batch %d failed: %s", self.current_batch, e)
                    self.errors.append(f"Batch {self.current_batch}: {str(e)}")

                self.processed_files += len(batch)

            # Step 5: Build candidates filtered by minimum file count
            self._build_candidates(existing_tag_names)

        except Exception as e:
            logger.error("Explorer error: %s", e, exc_info=True)
            self.errors.append(f"Explorer error: {str(e)}")
        finally:
            elapsed = time.time() - explore_start
            logger.info("Tag exploration finished: %d candidates, %d existing tags assigned in %.1fs",
                        len(self.candidates), self.existing_tags_assigned, elapsed)
            self.is_running = False
            db.close()

    def _apply_tag(self, db, file_rec, tag_name, tag_cache, source="auto", confidence=0.8):
        """Apply a single tag to a file, updating counters. Returns True if new."""
        if not file_rec:
            return False
        tag = tag_cache.get(tag_name)
        if not tag:
            return False

        existing_ft = db.query(FileTag).filter(
            FileTag.file_id == file_rec.id,
            FileTag.tag_id == tag.id,
        ).first()
        if existing_ft:
            return False

        db.add(FileTag(
            file_id=file_rec.id,
            tag_id=tag.id,
            source=source,
            confidence=confidence,
        ))
        tag.file_count = db.query(FileTag).filter(
            FileTag.tag_id == tag.id
        ).count() + 1
        db.commit()
        self.existing_tags_assigned += 1
        if file_rec.id not in self._existing_files_set:
            self._existing_files_set.add(file_rec.id)
            self.existing_files_tagged += 1
        return True

    async def _discover_new_tags_batch(self, files_info, existing_tags, discovered_names=None):
        """Ask LLM to propose new tag categories (not existing ones)."""
        existing_list = "\n".join(
            f"- {name}: {info['description']}" for name, info in existing_tags.items()
        )

        discovered_block = ""
        if discovered_names:
            discovered_list = "\n".join(f"- {name}" for name in sorted(discovered_names))
            discovered_block = f"""
ALREADY DISCOVERED NEW CATEGORIES (do NOT propose these again):
{discovered_list}
"""

        file_entries = []
        for i, info in enumerate(files_info):
            preview = info["text_preview"][:1500]
            path = info.get("relative_path", "")
            file_entries.append(f"FILE {i+1}: {info['filename']} (path: {path})\n{preview}\n---")
        files_block = "\n".join(file_entries)

        prompt = f"""You are a geotechnical and civil engineering document librarian.

These categories ALREADY EXIST (do NOT propose them):
{existing_list}
{discovered_block}
Analyze the following documents and propose NEW categories for topics NOT covered by any existing or already-discovered category above.

Rules:
- New categories must be specific to civil/geotechnical engineering
- Each new category needs a slug_name (lowercase, underscores), display_name, and brief description
- List which filenames belong to each new category
- Do NOT propose categories that overlap with existing ones

{files_block}

Reply with ONLY a JSON object in this exact format:
{{
  "new_tags": {{
    "slug_name": {{
      "display_name": "Human Readable Name",
      "description": "Brief description of category",
      "filenames": ["filename.pdf"]
    }}
  }}
}}

If no new categories can be identified, reply with: {{"new_tags": {{}}}}"""

        response = await chat_completion([
            {"role": "system", "content": "You are a geotechnical engineering document classification expert. Reply only with valid JSON."},
            {"role": "user", "content": prompt},
        ], max_tokens=1024)

        response = response.strip()
        if response.startswith("```"):
            response = response.split("\n", 1)[1].rsplit("```", 1)[0].strip()

        # Strip <think> blocks from Qwen3
        if "<think>" in response:
            response = re.sub(r"<think>.*?</think>", "", response, flags=re.DOTALL).strip()

        parsed = json.loads(response)
        if not isinstance(parsed, dict):
            return {}

        return parsed.get("new_tags", {})

    def _build_candidates(self, existing_tag_names):
        """Filter raw suggestions by minimum file count and assign colors."""
        color_idx = 0
        self.candidates = []

        for tag_name, info in sorted(
            self._raw_suggestions.items(),
            key=lambda x: len(x[1]["files"]),
            reverse=True,
        ):
            if tag_name in existing_tag_names:
                continue
            file_count = len(info["files"])
            if file_count < MIN_FILES_PER_TAG:
                continue

            color = EXPLORE_COLORS[color_idx % len(EXPLORE_COLORS)]
            color_idx += 1

            self.candidates.append({
                "name": tag_name,
                "display_name": info["display_name"],
                "description": info["description"],
                "color": color,
                "file_count": file_count,
                "filenames": list(info["files"]),
            })
