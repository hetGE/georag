//! Spreadsheets through calamine, CSV through csv. One `[Sheet: name]`
//! marker per sheet, rows as tab-separated cells, empty rows dropped.

use std::fmt::Write as _;
use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};

use crate::{ExtractError, Extracted, Result, io_err};

pub fn extract_workbook(path: &Path) -> Result<Extracted> {
    let mut workbook = open_workbook_auto(path).map_err(|e| ExtractError::Malformed {
        path: path.to_path_buf(),
        kind: "spreadsheet",
        reason: e.to_string(),
    })?;
    let names: Vec<String> = workbook.sheet_names().to_vec();
    let mut text = String::new();
    let mut units = 0u32;
    for name in names {
        let range = match workbook.worksheet_range(&name) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("sheet {name} of {}: {e}", path.display());
                continue;
            }
        };
        units += 1;
        let _ = writeln!(text, "[Sheet: {name}]");
        for row in range.rows() {
            let cells: Vec<String> = row.iter().map(cell_text).collect();
            if cells.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            text.push_str(cells.join("\t").trim_end());
            text.push('\n');
        }
        text.push('\n');
    }
    Ok(Extracted {
        text,
        units,
        unit_kind: "sheet",
    })
}

fn cell_text(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::Error(e) => format!("{e:?}"),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
    }
}

pub fn extract_csv(path: &Path) -> Result<Extracted> {
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let delimiter = if path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("tsv"))
        .unwrap_or(false)
    {
        b'\t'
    } else {
        b','
    };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes.as_slice());
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut text = format!("[Sheet: {name}]\n");
    for record in rdr.records() {
        let record = record.map_err(|e| ExtractError::Malformed {
            path: path.to_path_buf(),
            kind: "CSV",
            reason: e.to_string(),
        })?;
        let cells: Vec<&str> = record.iter().collect();
        if cells.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        text.push_str(&cells.join("\t"));
        text.push('\n');
    }
    Ok(Extracted {
        text,
        units: 1,
        unit_kind: "sheet",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_rows_become_tab_lines() {
        let dir = std::env::temp_dir().join(format!("georag-csv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("results.csv");
        std::fs::write(&p, "BH,depth,su\nBH-1,5,80\n,,\nBH-2,10,95\n").unwrap();
        let e = extract_csv(&p).unwrap();
        assert_eq!(
            e.text,
            "[Sheet: results]\nBH\tdepth\tsu\nBH-1\t5\t80\nBH-2\t10\t95\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
