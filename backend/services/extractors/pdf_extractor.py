"""PDF text extraction using pdfplumber."""
import pdfplumber


async def extract_pdf(file_path: str) -> str:
    """Extract text from a PDF file."""
    text_parts = []
    try:
        with pdfplumber.open(file_path) as pdf:
            for i, page in enumerate(pdf.pages):
                try:
                    page_text = page.extract_text()
                    if page_text:
                        text_parts.append(f"[Page {i + 1}]\n{page_text}")
                except Exception:
                    continue
    except Exception as e:
        raise Exception(f"PDF extraction failed: {e}")

    return "\n\n".join(text_parts)
