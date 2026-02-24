"""Plain text, HTML, RTF extraction."""
from pathlib import Path
import re


async def extract_text(file_path: str) -> str:
    """Extract text from plain text files, HTML, RTF, CSV, Markdown."""
    path = Path(file_path)

    try:
        # Try UTF-8 first, then latin-1 as fallback
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            text = path.read_text(encoding="latin-1")

        ext = path.suffix.lower()

        if ext in (".html", ".htm"):
            # Strip HTML tags
            text = re.sub(r"<script[^>]*>.*?</script>", "", text, flags=re.DOTALL)
            text = re.sub(r"<style[^>]*>.*?</style>", "", text, flags=re.DOTALL)
            text = re.sub(r"<[^>]+>", " ", text)
            text = re.sub(r"\s+", " ", text).strip()

        return text
    except Exception as e:
        raise Exception(f"Text extraction failed: {e}")
