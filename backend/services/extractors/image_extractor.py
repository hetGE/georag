"""Image description using a vision-capable chat model served by llama-server (requires --mmproj)."""
import base64
from pathlib import Path

from backend.services.llm_client import vision_describe


async def extract_image(file_path: str) -> str:
    """Extract description from an image using vision model."""
    try:
        path = Path(file_path)
        if path.stat().st_size > 10 * 1024 * 1024:  # Skip images > 10MB
            return f"Image file: {path.name} (too large for vision processing)"

        with open(file_path, "rb") as f:
            image_b64 = base64.b64encode(f.read()).decode("utf-8")

        description = await vision_describe(
            image_b64,
            prompt="Describe this geotechnical engineering image, diagram, or figure in detail. "
                   "Include any visible text, labels, measurements, soil layers, or structural elements."
        )
        return f"[Image: {path.name}]\n{description}"
    except Exception as e:
        return f"Image file: {Path(file_path).name} (vision extraction failed: {e})"
