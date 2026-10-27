//! Plain text, markdown, HTML and RTF.

use std::path::Path;

use crate::{Extracted, Result, io_err};

pub fn extract_plain(path: &Path) -> Result<Extracted> {
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    Ok(Extracted {
        text,
        units: 1,
        unit_kind: "file",
    })
}

pub fn extract_html(path: &Path) -> Result<Extracted> {
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let text = html2text::config::plain_no_decorate()
        .string_from_read(bytes.as_slice(), 100)
        .map_err(|e| crate::ExtractError::Malformed {
            path: path.to_path_buf(),
            kind: "HTML",
            reason: e.to_string(),
        })?;
    Ok(Extracted {
        text,
        units: 1,
        unit_kind: "file",
    })
}

/// A small RTF reader: drops control words and groups, keeps text and the
/// paragraph breaks. Enough for reports saved as RTF.
pub fn extract_rtf(path: &Path) -> Result<Extracted> {
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let src = String::from_utf8_lossy(&bytes);
    Ok(Extracted {
        text: rtf_to_text(&src),
        units: 1,
        unit_kind: "file",
    })
}

pub fn rtf_to_text(src: &str) -> String {
    let mut out = String::new();
    let mut chars = src.chars().peekable();
    let mut depth_skip: Option<usize> = None;
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '{' => depth += 1,
            '}' => {
                if depth_skip == Some(depth) {
                    depth_skip = None;
                }
                depth = depth.saturating_sub(1);
            }
            '\\' => {
                let mut word = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphabetic() {
                        word.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if word.is_empty() {
                    // Escaped char or control symbol.
                    match chars.next() {
                        Some('\'') => {
                            let h: String = chars.by_ref().take(2).collect();
                            if let Ok(b) = u8::from_str_radix(&h, 16) {
                                out.push(b as char);
                            }
                        }
                        Some('*') => depth_skip = Some(depth),
                        Some('\\') | Some('{') | Some('}') => out.push(c),
                        Some('~') => out.push(' '),
                        Some('\n') | Some('\r') => out.push('\n'),
                        _ => {}
                    }
                    continue;
                }
                // Numeric parameter and the optional delimiting space.
                let mut param = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_digit() || (n == '-' && param.is_empty()) {
                        param.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                match word.as_str() {
                    "par" | "line" | "sect" | "page" => out.push('\n'),
                    "tab" | "cell" => out.push('\t'),
                    "row" => out.push('\n'),
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "object"
                    | "header" | "footer" | "themedata" | "latentstyles" | "datastore" => {
                        depth_skip = Some(depth);
                    }
                    "u" => {
                        if let Ok(code) = param.parse::<i32>() {
                            let code = if code < 0 { code + 65536 } else { code };
                            if let Some(ch) = char::from_u32(code as u32) {
                                out.push(ch);
                            }
                            // Skip the fallback character that follows.
                            chars.next();
                        }
                    }
                    _ => {}
                }
            }
            '\r' | '\n' => {}
            _ => {
                if depth_skip.is_none() {
                    out.push(c);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtf_keeps_text_and_paragraphs() {
        // The unicode escape is assembled at runtime so no tool rewrites it.
        let bs = '\\';
        let src = format!(
            "{{{bs}rtf1{bs}ansi{{{bs}fonttbl{{{bs}f0 Arial;}}}}{bs}f0{bs}fs24 Pile capacity 1200 kN.{bs}par Second {bs}'e9 line{bs}u8364? end.}}"
        );
        let t = rtf_to_text(&src);
        assert!(t.contains("Pile capacity 1200 kN."), "{t:?}");
        assert!(t.contains("\nSecond \u{e9} line\u{20ac} end."), "{t:?}");
        assert!(!t.contains("Arial"));
    }

    #[test]
    fn html_to_text_strips_tags() {
        let dir = std::env::temp_dir().join(format!("georag-html-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.html");
        std::fs::write(
            &p,
            "<html><body><h1>Report</h1><p>Clay with <b>su</b> 80 kPa.</p></body></html>",
        )
        .unwrap();
        let e = extract_html(&p).unwrap();
        assert!(e.text.contains("Report"));
        assert!(e.text.contains("su 80 kPa"));
        assert!(!e.text.contains("<p>"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
