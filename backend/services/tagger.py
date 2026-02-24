"""Auto-tagging: folder heuristic + LLM classification."""
from backend.config import DEFAULT_TAGS
from backend.services.llm_client import chat_completion

# Folder name keywords to tag mappings
FOLDER_HINTS = {
    "pile": "piling",
    "piling": "piling",
    "bored pile": "piling",
    "driven pile": "piling",
    "cfa": "piling",
    "diaphragm": "diaphragm_wall",
    "dwall": "diaphragm_wall",
    "d-wall": "diaphragm_wall",
    "slurry wall": "diaphragm_wall",
    "barrette": "diaphragm_wall",
    "ground improvement": "ground_improvement",
    "soil improvement": "ground_improvement",
    "grouting": "ground_improvement",
    "jet grout": "ground_improvement",
    "dsm": "ground_improvement",
    "soil mixing": "ground_improvement",
    "compaction": "ground_improvement",
    "pvd": "ground_improvement",
    "anchor": "ground_anchors",
    "soil nail": "ground_anchors",
    "tieback": "ground_anchors",
    "rock bolt": "ground_anchors",
    "excavation": "deep_excavation",
    "braced": "deep_excavation",
    "cofferdam": "deep_excavation",
    "sheet pile": "deep_excavation",
    "strut": "deep_excavation",
    "tunnel": "tunneling",
    "tbm": "tunneling",
    "natm": "tunneling",
    "earthquake": "earthquake",
    "seismic": "earthquake",
    "liquefaction": "earthquake",
    "lab test": "lab_testing",
    "laboratory": "lab_testing",
    "triaxial": "lab_testing",
    "consolidation": "lab_testing",
    "oedometer": "lab_testing",
    "spt": "insitu_testing",
    "cpt": "insitu_testing",
    "in-situ": "insitu_testing",
    "in situ": "insitu_testing",
    "pressuremeter": "insitu_testing",
    "vane shear": "insitu_testing",
    "plaxis": "finite_element_analysis",
    "fem": "finite_element_analysis",
    "fea": "finite_element_analysis",
    "finite element": "finite_element_analysis",
    "numerical": "finite_element_analysis",
}


def tag_by_folder(relative_path: str) -> list[tuple[str, float]]:
    """Assign tags based on folder path keywords.
    Returns list of (tag_name, confidence) tuples."""
    path_lower = relative_path.lower()
    tags_found = set()

    for keyword, tag_name in FOLDER_HINTS.items():
        if keyword in path_lower:
            tags_found.add(tag_name)

    return [(tag, 0.7) for tag in tags_found]


async def tag_by_llm(text_preview: str, filename: str) -> list[tuple[str, float]]:
    """Use LLM to classify document into tags.
    Returns list of (tag_name, confidence) tuples."""
    tag_names = [t["name"] for t in DEFAULT_TAGS]
    tag_descriptions = "\n".join(
        f"- {t['name']}: {t['description']}" for t in DEFAULT_TAGS
    )

    prompt = f"""Classify this geotechnical engineering document into one or more of these categories.

Available categories:
{tag_descriptions}

Document filename: {filename}
Document text (first ~2000 chars):
{text_preview[:2000]}

Reply with ONLY a JSON array of category names that apply. Example: ["piling", "deep_excavation"]
If none apply clearly, reply with an empty array: []"""

    try:
        response = await chat_completion([
            {"role": "system", "content": "You are a geotechnical engineering document classifier. Reply only with a JSON array."},
            {"role": "user", "content": prompt},
        ], max_tokens=200)

        # Parse JSON from response
        import json
        # Try to extract JSON array from response
        response = response.strip()
        if response.startswith("```"):
            response = response.split("\n", 1)[1].rsplit("```", 1)[0].strip()

        tags = json.loads(response)
        if isinstance(tags, list):
            return [(t, 0.8) for t in tags if t in tag_names]
    except Exception:
        pass

    return []
