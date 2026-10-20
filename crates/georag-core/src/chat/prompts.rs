//! Prompt texts. Citation tags are `[S<n>]`, numbered once across a run.

use crate::retrieval::Candidate;

pub const SYSTEM_BASE: &str = "You are GeoRAG, an assistant for geotechnical engineers. Answer from the numbered sources below. After each statement a source supports, cite it with its tag, for example [S3]. Keep the tags exactly as given. If the sources do not cover part of the question, say so plainly instead of guessing. Use SI units as the sources do.";

pub fn pass_system(i: u32, j: u32, n: u32) -> String {
    format!(
        "{SYSTEM_BASE}\n\nYou are reading sources {i} to {j} of {n} for this question. Answer only from these sources. End with one line listing what these sources do not cover."
    )
}

pub const FINAL_SYSTEM: &str = "Combine the following partial answers into one answer to the question. Reconcile disagreements, say where sources differ, keep every citation tag unchanged, and list sources that were skipped or not read.";

/// `[S<n>] <rel_path> <locator>` followed by the text.
pub fn format_source(tag: u32, c: &Candidate) -> String {
    let locator = c.locator.as_deref().unwrap_or("");
    format!("[S{tag}] {} {locator}\n{}\n", c.rel_path, c.text.trim())
}

pub fn sources_block(tagged: &[(u32, &Candidate)]) -> String {
    let mut out = String::new();
    for (tag, c) in tagged {
        out.push_str(&format_source(*tag, c));
        out.push('\n');
    }
    out
}

/// Tags cited in an answer, in first-seen order. Accepts `[S3]` and lists
/// such as `[S1, S2]` or `[S1][S2]`.
pub fn cited_tags(answer: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        let inner = &after[..close];
        let mut valid = true;
        let mut found = Vec::new();
        for part in inner.split(',') {
            let part = part.trim();
            match part.strip_prefix('S').and_then(|n| n.parse::<u32>().ok()) {
                Some(n) if !part[1..].is_empty() => found.push(n),
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        if valid {
            for n in found {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        rest = &after[close + 1..];
    }
    out
}

/// Remove `<think>...</think>` blocks (stored history is kept, prompts are not).
pub fn strip_think(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_citations_once_each() {
        assert_eq!(
            cited_tags("Capacity 1200 kN [S3], see also [S1] and [S3]. [S12]"),
            vec![3, 1, 12]
        );
        assert_eq!(
            cited_tags("lists [S1, S2, S3] and [S4][S5]"),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(
            cited_tags("no tags [S] [Sx] [1] [see S2]"),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn strips_think_blocks() {
        assert_eq!(strip_think("<think>\nhmm\n</think>\n\nAnswer"), "Answer");
        assert_eq!(strip_think("plain"), "plain");
        assert_eq!(strip_think("<think>unterminated"), "");
    }
}
