"""Auto-tagging: folder/filename heuristic + LLM classification."""
import json
import re

from backend.config import DEFAULT_TAGS
from backend.services.llm_client import chat_completion

# Keywords to tag mappings (checked against folder path AND filename)
HEURISTIC_HINTS = {
    "pile": "piling",
    "piling": "piling",
    "bored pile": "piling",
    "driven pile": "piling",
    "cfa": "piling",
    "pile load": "piling",
    "pile cap": "piling",
    "micropile": "piling",
    "diaphragm": "diaphragm_wall",
    "dwall": "diaphragm_wall",
    "d-wall": "diaphragm_wall",
    "d wall": "diaphragm_wall",
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
    "stone column": "ground_improvement",
    "vibro": "ground_improvement",
    "surcharge": "ground_improvement",
    "preloading": "ground_improvement",
    "anchor": "ground_anchors",
    "soil nail": "ground_anchors",
    "tieback": "ground_anchors",
    "rock bolt": "ground_anchors",
    "ground anchor": "ground_anchors",
    "prestress": "ground_anchors",
    "excavation": "deep_excavation",
    "braced": "deep_excavation",
    "cofferdam": "deep_excavation",
    "sheet pile": "deep_excavation",
    "strut": "deep_excavation",
    "retaining": "deep_excavation",
    "shoring": "deep_excavation",
    "eow": "deep_excavation",
    "earth retaining": "deep_excavation",
    "tunnel": "tunneling",
    "tbm": "tunneling",
    "natm": "tunneling",
    "cut and cover": "tunneling",
    "earthquake": "earthquake",
    "seismic": "earthquake",
    "liquefaction": "earthquake",
    "dynamic analysis": "earthquake",
    "lab test": "lab_testing",
    "laboratory": "lab_testing",
    "triaxial": "lab_testing",
    "consolidation": "lab_testing",
    "oedometer": "lab_testing",
    "direct shear": "lab_testing",
    "atterberg": "lab_testing",
    "grain size": "lab_testing",
    "sieve": "lab_testing",
    "ucs": "lab_testing",
    "unconfined": "lab_testing",
    "spt": "insitu_testing",
    "cpt": "insitu_testing",
    "in-situ": "insitu_testing",
    "in situ": "insitu_testing",
    "pressuremeter": "insitu_testing",
    "vane shear": "insitu_testing",
    "bore log": "insitu_testing",
    "borehole": "insitu_testing",
    "bore hole": "insitu_testing",
    "field test": "insitu_testing",
    "plate load": "insitu_testing",
    "plaxis": "finite_element_analysis",
    "fem": "finite_element_analysis",
    "fea": "finite_element_analysis",
    "finite element": "finite_element_analysis",
    "numerical": "finite_element_analysis",
    "flac": "finite_element_analysis",
    "abaqus": "finite_element_analysis",
    "foundation": "piling",
    "bearing capacity": "piling",
    "settlement": "piling",
    "slope": "ground_improvement",
    "landslide": "ground_improvement",
    "embankment": "ground_improvement",
}


def tag_by_heuristic(relative_path: str, filename: str) -> list[tuple[str, float]]:
    """Assign tags based on folder path AND filename keywords.
    Returns list of (tag_name, confidence) tuples."""
    path_lower = relative_path.lower()
    # Normalize filename: remove extension, replace separators with spaces
    name_lower = re.sub(r"[_\-.]", " ", filename.rsplit(".", 1)[0].lower())
    combined = f"{path_lower} {name_lower}"

    tags_found = set()
    for keyword, tag_name in HEURISTIC_HINTS.items():
        if keyword in combined:
            tags_found.add(tag_name)

    return [(tag, 0.7) for tag in tags_found]


# Keep old name as alias for backward compatibility
tag_by_folder = tag_by_heuristic


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
        response = response.strip()
        if response.startswith("```"):
            response = response.split("\n", 1)[1].rsplit("```", 1)[0].strip()

        tags = json.loads(response)
        if isinstance(tags, list):
            return [(t, 0.8) for t in tags if t in tag_names]
    except Exception:
        pass

    return []


async def tag_batch_by_llm(files_info: list[dict]) -> dict[str, list[tuple[str, float]]]:
    """Classify multiple files in one LLM call.
    files_info: [{"filename": str, "text_preview": str}, ...]
    Returns: {"filename": [(tag_name, confidence), ...], ...}
    """
    tag_names = [t["name"] for t in DEFAULT_TAGS]
    tag_descriptions = "\n".join(
        f"- {t['name']}: {t['description']}" for t in DEFAULT_TAGS
    )

    file_entries = []
    for i, info in enumerate(files_info):
        preview = info["text_preview"][:1500]
        file_entries.append(f"FILE {i+1}: {info['filename']}\n{preview}\n---")

    files_block = "\n".join(file_entries)

    prompt = f"""Classify each of these geotechnical engineering documents into one or more categories.

Available categories:
{tag_descriptions}

{files_block}

Reply with ONLY a JSON object mapping each filename to its category array.
Example: {{"report.pdf": ["piling", "deep_excavation"], "log.xlsx": ["insitu_testing"]}}
If none apply for a file, use an empty array."""

    result = {info["filename"]: [] for info in files_info}

    try:
        response = await chat_completion([
            {"role": "system", "content": "You are a geotechnical engineering document classifier. Reply only with a JSON object."},
            {"role": "user", "content": prompt},
        ], max_tokens=500)

        response = response.strip()
        if response.startswith("```"):
            response = response.split("\n", 1)[1].rsplit("```", 1)[0].strip()

        parsed = json.loads(response)
        if isinstance(parsed, dict):
            for filename, tags in parsed.items():
                if isinstance(tags, list) and filename in result:
                    result[filename] = [(t, 0.8) for t in tags if t in tag_names]
    except Exception:
        pass

    return result
