//! Import from v1 (handoff section 10): the `_0RAG` folder, its
//! `metadata.db`, or a `.georag` backup archive. Never writes to the source.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use georag_store::v1::{V1Dump, V1File, V1FileTag, V1Tag};
use georag_store::{FileState, NewMessage, Source, Store};
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::home::Home;
use crate::tags::{TagDef, TagFile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum InputKind {
    Folder,
    MetadataDb,
    Archive,
}

/// Where the data came from and where the documents live.
#[derive(Debug, Clone, Serialize)]
pub struct Located {
    pub kind: InputKind,
    pub source: PathBuf,
    /// Default root: the parent of `_0RAG`, when known.
    pub default_root: Option<PathBuf>,
}

/// Work out what `path` is and load the dump.
pub fn locate(path: &Path) -> Result<(Located, V1Dump)> {
    let path = path.canonicalize().map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if path.is_dir() {
        let db = if path.file_name().map(|n| n == "_0RAG").unwrap_or(false) {
            path.join("data/metadata.db")
        } else {
            path.join("_0RAG/data/metadata.db")
        };
        if !db.is_file() {
            return Err(CoreError::Invalid(format!(
                "{} holds no _0RAG/data/metadata.db: point at the v1 folder, the database, or a .georag archive",
                path.display()
            )));
        }
        let rag_dir = db.parent().and_then(Path::parent).map(Path::to_path_buf);
        let default_root = rag_dir
            .as_ref()
            .and_then(|r| r.parent())
            .map(Path::to_path_buf);
        let dump = georag_store::v1::read_metadata_db(&db)?;
        return Ok((
            Located {
                kind: InputKind::Folder,
                source: db,
                default_root,
            },
            dump,
        ));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "georag" || ext == "zip" {
        let dump = read_archive(&path)?;
        return Ok((
            Located {
                kind: InputKind::Archive,
                source: path,
                default_root: None,
            },
            dump,
        ));
    }
    let dump = georag_store::v1::read_metadata_db(&path)?;
    // metadata.db sits in _0RAG/data/.
    let default_root = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    Ok((
        Located {
            kind: InputKind::MetadataDb,
            source: path,
            default_root,
        },
        dump,
    ))
}

/// `.georag` archive: manifest.json, files.jsonl, tags.json, file_tags.jsonl;
/// vectors (`*.npz`) are ignored.
pub fn read_archive(path: &Path) -> Result<V1Dump> {
    let file = std::fs::File::open(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        CoreError::Invalid(format!("{} is not a .georag archive: {e}", path.display()))
    })?;
    let mut read = |name: &str| -> Result<Option<String>> {
        match zip.by_name(name) {
            Ok(mut entry) => {
                let mut s = String::new();
                entry
                    .read_to_string(&mut s)
                    .map_err(|source| CoreError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
                Ok(Some(s))
            }
            Err(_) => Ok(None),
        }
    };
    let files_text = read("files.jsonl")?
        .ok_or_else(|| CoreError::Invalid(format!("{} has no files.jsonl", path.display())))?;
    let tags_text = read("tags.json")?.unwrap_or_else(|| "[]".into());
    let file_tags_text = read("file_tags.jsonl")?.unwrap_or_default();
    let mut dump = V1Dump::default();
    for line in files_text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line)?;
        dump.files.push(V1File {
            id: v["id"].as_i64().unwrap_or(0),
            relative_path: v["relative_path"].as_str().unwrap_or("").to_string(),
            filename: v["filename"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    Path::new(v["relative_path"].as_str().unwrap_or(""))
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default()
                }),
            extension: v["extension"].as_str().map(str::to_string),
            size_bytes: v["size_bytes"].as_i64(),
            modified_time: v["modified_time"].as_f64(),
            content_hash: v["content_hash"].as_str().map(str::to_string),
            scan_status: v["scan_status"].as_str().map(str::to_string),
            extracted_text_preview: v["extracted_text_preview"].as_str().map(str::to_string),
            chunk_count: v["chunk_count"].as_i64(),
        });
    }
    let tags: Vec<serde_json::Value> = serde_json::from_str(&tags_text)?;
    for v in tags {
        dump.tags.push(V1Tag {
            id: v["id"].as_i64().unwrap_or(0),
            name: v["name"].as_str().unwrap_or("").to_string(),
            display_name: v["display_name"].as_str().unwrap_or("").to_string(),
            description: v["description"].as_str().map(str::to_string),
            color: v["color"].as_str().map(str::to_string),
        });
    }
    for line in file_tags_text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line)?;
        dump.file_tags.push(V1FileTag {
            file_id: v["file_id"].as_i64().unwrap_or(0),
            tag_id: v["tag_id"].as_i64().unwrap_or(0),
            source: v["source"].as_str().map(str::to_string),
            confidence: v["confidence"].as_f64(),
        });
    }
    Ok(dump)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Preview {
    pub root: PathBuf,
    pub files: usize,
    pub files_on_disk: usize,
    pub files_missing: usize,
    pub files_failed_in_v1: usize,
    pub tags: usize,
    pub tags_builtin: usize,
    pub tags_custom: Vec<String>,
    pub file_tags: usize,
    pub conversations: usize,
    pub messages: usize,
    pub already_present: usize,
}

/// Map v1 tag sources to v2 (`manual`, `rule`, `llm`).
pub fn map_source(v1: Option<&str>) -> &'static str {
    match v1.unwrap_or("auto") {
        "manual" => "manual",
        "folder_hint" => "rule",
        _ => "llm",
    }
}

/// Count what the import would do.
pub fn preview(store: &Store, dump: &V1Dump, root: &Path, defs: &TagFile) -> Result<Preview> {
    let mut p = Preview {
        root: root.to_path_buf(),
        ..Default::default()
    };
    p.files = dump.files.len();
    let existing: HashMap<String, i64> =
        match store.roots()?.iter().find(|r| Path::new(&r.path) == root) {
            Some(r) => store
                .files_query(&georag_store::FileQuery {
                    root_id: Some(r.id),
                    ..Default::default()
                })?
                .into_iter()
                .map(|f| (f.rel_path, f.id))
                .collect(),
            None => HashMap::new(),
        };
    for f in &dump.files {
        if root.join(&f.relative_path).is_file() {
            p.files_on_disk += 1;
        } else {
            p.files_missing += 1;
        }
        if f.scan_status.as_deref() == Some("failed") {
            p.files_failed_in_v1 += 1;
        }
        if existing.contains_key(&f.relative_path) {
            p.already_present += 1;
        }
    }
    p.tags = dump.tags.len();
    for t in &dump.tags {
        if defs.get(&t.name).is_some() {
            p.tags_builtin += 1;
        } else {
            p.tags_custom.push(t.name.clone());
        }
    }
    p.file_tags = dump.file_tags.len();
    p.conversations = dump.conversations.iter().filter(|c| !c.deleted).count();
    p.messages = dump.messages.len();
    Ok(p)
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub with_chats: bool,
    /// Queue the tag stage even for files that carry imported tags.
    pub retag: bool,
    pub root_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub root: String,
    pub files_imported: usize,
    pub files_merged: usize,
    pub files_missing: Vec<String>,
    pub tags_added_to_file: Vec<String>,
    pub file_tags: usize,
    pub conversations: usize,
    pub messages: usize,
    pub jobs_queued: usize,
}

/// Perform the import. Idempotent: files merge by (root, rel_path), tags
/// by name, file tags by (file, tag).
pub fn run(
    home: &Home,
    store: &Store,
    dump: &V1Dump,
    root: &Path,
    opts: &ImportOptions,
) -> Result<ImportReport> {
    let root = root.canonicalize().map_err(|source| CoreError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut report = ImportReport::default();

    // Root.
    let root_row = match store
        .roots()?
        .into_iter()
        .find(|r| Path::new(&r.path) == root)
    {
        Some(r) => r,
        None => {
            let name = opts.root_name.clone().unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "library".into())
            });
            store.root_add(&name, &root)?
        }
    };
    report.root = root_row.name.clone();

    // Tags: built-ins by name, custom appended to tags.toml.
    let mut defs = TagFile::load_or_create(home)?;
    for t in &dump.tags {
        if defs.get(&t.name).is_none() {
            defs.upsert(TagDef {
                name: t.name.clone(),
                display_name: t.display_name.clone(),
                color: t.color.clone(),
                description: t.description.clone(),
                patterns: Vec::new(),
            });
            report.tags_added_to_file.push(t.name.clone());
        }
    }
    if !report.tags_added_to_file.is_empty() {
        defs.save(home)?;
    }
    defs.seed(store)?;
    let tag_names: HashMap<i64, String> =
        dump.tags.iter().map(|t| (t.id, t.name.clone())).collect();

    // Files.
    let existing: HashMap<String, i64> = store
        .files_query(&georag_store::FileQuery {
            root_id: Some(root_row.id),
            ..Default::default()
        })?
        .into_iter()
        .map(|f| (f.rel_path, f.id))
        .collect();
    let mut id_map: HashMap<i64, i64> = HashMap::new();
    for f in &dump.files {
        let rel = f.relative_path.replace('\\', "/");
        let on_disk = root.join(&rel).is_file();
        let id = match existing.get(&rel) {
            Some(id) => {
                report.files_merged += 1;
                *id
            }
            None => {
                report.files_imported += 1;
                let state = if on_disk {
                    FileState::Imported
                } else {
                    FileState::Skipped
                };
                let id = store.file_upsert(root_row.id, &rel, f.size_bytes, state)?;
                if !on_disk {
                    store.file_set_state(id, FileState::Skipped, Some("missing at import"))?;
                    report.files_missing.push(rel.clone());
                } else if f.scan_status.as_deref() == Some("failed") {
                    store.file_set_v1_note(id, "no text in v1")?;
                }
                if let Some(h) = &f.content_hash
                    && h.len() == 64
                {
                    // sha256 from v1: kept for information; the first scan recomputes blake3.
                    store.file_set_v1_note(id, &format!("v1 sha256 {h}"))?;
                }
                id
            }
        };
        id_map.insert(f.id, id);
    }

    // File tags.
    for ft in &dump.file_tags {
        let (Some(file_id), Some(tag)) = (id_map.get(&ft.file_id), tag_names.get(&ft.tag_id))
        else {
            continue;
        };
        store.file_tag_add(
            *file_id,
            tag,
            map_source(ft.source.as_deref()),
            ft.confidence,
            true,
        )?;
        report.file_tags += 1;
    }

    // Chats.
    if opts.with_chats {
        let mut conv_map: HashMap<i64, i64> = HashMap::new();
        for c in dump.conversations.iter().filter(|c| !c.deleted) {
            let created = store.conversation_create(c.title.as_deref(), &c.selected_tags)?;
            conv_map.insert(c.id, created.id);
            report.conversations += 1;
        }
        for m in &dump.messages {
            let Some(conv_id) = conv_map.get(&m.conversation_id) else {
                continue;
            };
            if m.role != "user" && m.role != "assistant" {
                continue;
            }
            let sources: Vec<Source> = m
                .sources
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .enumerate()
                        .filter_map(|(i, s)| {
                            let rel = s
                                .get("relative_path")
                                .or_else(|| s.get("file"))
                                .or_else(|| s.get("source"))
                                .and_then(|v| v.as_str())?;
                            let page = s
                                .get("page")
                                .and_then(|p| p.as_i64())
                                .map(|p| format!("p.{p}"));
                            Some(Source {
                                file_id: 0,
                                rel_path: rel.to_string(),
                                locator: page,
                                chunk_id: 0,
                                score: 0.0,
                                tag: i as u32 + 1,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            store.message_add(&NewMessage {
                conversation_id: *conv_id,
                role: &m.role,
                content: &m.content,
                sources: &sources,
                mode: Some("v1"),
                cut_off: false,
            })?;
            report.messages += 1;
        }
    }

    // Queue extraction for every imported file that is on disk.
    for (_, id) in id_map {
        let Some(file) = store.file_get(id)? else {
            continue;
        };
        if file.state == FileState::Skipped {
            continue;
        }
        if opts.retag {
            // Drop imported tags so the tag stage runs again.
            for t in store.file_tags(id)? {
                if t.imported {
                    store.file_tag_remove(id, &t.tag)?;
                }
            }
        }
        if file.state == FileState::Imported || file.state == FileState::New {
            store.job_enqueue("extract", Some(id), None)?;
            report.jobs_queued += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use georag_store::v1::{V1Conversation, V1Message, write_fixture_db};

    fn fixture(dir: &Path) -> (PathBuf, V1Dump) {
        let root = dir.join("Engineering");
        std::fs::create_dir_all(root.join("_0RAG/data")).unwrap();
        std::fs::create_dir_all(root.join("Piles")).unwrap();
        std::fs::write(root.join("Piles/a.txt"), "bored pile capacity 1200 kN").unwrap();
        let dump = V1Dump {
            files: vec![
                V1File {
                    id: 1,
                    relative_path: "Piles/a.txt".into(),
                    filename: "a.txt".into(),
                    extension: Some("txt".into()),
                    size_bytes: Some(27),
                    modified_time: Some(1.0),
                    content_hash: None,
                    scan_status: Some("processed".into()),
                    extracted_text_preview: None,
                    chunk_count: Some(1),
                },
                V1File {
                    id: 2,
                    relative_path: "Gone/b.pdf".into(),
                    filename: "b.pdf".into(),
                    extension: Some("pdf".into()),
                    size_bytes: Some(10),
                    modified_time: Some(1.0),
                    content_hash: None,
                    scan_status: Some("failed".into()),
                    extracted_text_preview: None,
                    chunk_count: Some(0),
                },
            ],
            tags: vec![
                V1Tag {
                    id: 1,
                    name: "piling".into(),
                    display_name: "Piling".into(),
                    description: None,
                    color: None,
                },
                V1Tag {
                    id: 2,
                    name: "custom_site".into(),
                    display_name: "Custom Site".into(),
                    description: Some("site docs".into()),
                    color: Some("#123456".into()),
                },
            ],
            file_tags: vec![
                V1FileTag {
                    file_id: 1,
                    tag_id: 1,
                    source: Some("folder_hint".into()),
                    confidence: Some(0.7),
                },
                V1FileTag {
                    file_id: 1,
                    tag_id: 2,
                    source: Some("manual".into()),
                    confidence: Some(1.0),
                },
                V1FileTag {
                    file_id: 2,
                    tag_id: 1,
                    source: Some("auto".into()),
                    confidence: Some(0.8),
                },
            ],
            conversations: vec![
                V1Conversation {
                    id: 1,
                    title: Some("Piles".into()),
                    selected_tags: vec!["piling".into()],
                    created_at: None,
                    deleted: false,
                },
                V1Conversation {
                    id: 2,
                    title: Some("Trash".into()),
                    selected_tags: vec![],
                    created_at: None,
                    deleted: true,
                },
            ],
            messages: vec![
                V1Message {
                    conversation_id: 1,
                    role: "user".into(),
                    content: "How much?".into(),
                    sources: serde_json::json!([]),
                    created_at: None,
                },
                V1Message {
                    conversation_id: 1,
                    role: "assistant".into(),
                    content: "1200 kN [Source 1]".into(),
                    sources: serde_json::json!([{"relative_path": "Piles/a.txt", "page": 3}]),
                    created_at: None,
                },
                V1Message {
                    conversation_id: 2,
                    role: "user".into(),
                    content: "gone".into(),
                    sources: serde_json::json!([]),
                    created_at: None,
                },
            ],
        };
        write_fixture_db(&root.join("_0RAG/data/metadata.db"), &dump).unwrap();
        (root, dump)
    }

    #[test]
    fn locate_preview_and_import_from_a_v1_folder() {
        let dir = std::env::temp_dir().join(format!("georag-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (root, _) = fixture(&dir);
        let home = Home::at(dir.join("GeoRAG"));
        home.init().unwrap();
        let store = Store::open(&home.db_path()).unwrap();

        let (located, dump) = locate(&root).unwrap();
        assert_eq!(located.kind, InputKind::Folder);
        assert_eq!(
            located.default_root.as_deref(),
            Some(root.canonicalize().unwrap().as_path())
        );
        let defs = TagFile::load_or_create(&home).unwrap();
        let p = preview(&store, &dump, &root, &defs).unwrap();
        assert_eq!(
            (
                p.files,
                p.files_on_disk,
                p.files_missing,
                p.files_failed_in_v1
            ),
            (2, 1, 1, 1)
        );
        assert_eq!(p.tags_custom, vec!["custom_site".to_string()]);
        assert_eq!(p.conversations, 1);

        let report = run(
            &home,
            &store,
            &dump,
            &root,
            &ImportOptions {
                with_chats: true,
                retag: false,
                root_name: Some("eng".into()),
            },
        )
        .unwrap();
        assert_eq!(report.files_imported, 2);
        assert_eq!(report.files_missing, vec!["Gone/b.pdf".to_string()]);
        assert_eq!(report.tags_added_to_file, vec!["custom_site".to_string()]);
        assert_eq!(report.file_tags, 3);
        assert_eq!(report.jobs_queued, 1);
        assert_eq!((report.conversations, report.messages), (1, 2));

        let files = store
            .files_query(&georag_store::FileQuery::default())
            .unwrap();
        let a = files.iter().find(|f| f.rel_path == "Piles/a.txt").unwrap();
        assert_eq!(a.state, FileState::Imported);
        let tags = store.file_tags(a.id).unwrap();
        assert!(tags.iter().all(|t| t.imported));
        assert_eq!(
            tags.iter().find(|t| t.tag == "piling").unwrap().source,
            "rule"
        );
        assert_eq!(
            tags.iter().find(|t| t.tag == "custom_site").unwrap().source,
            "manual"
        );
        let defs = TagFile::load_or_create(&home).unwrap();
        assert_eq!(
            defs.get("custom_site").unwrap().color.as_deref(),
            Some("#123456")
        );
        let convs = store.conversations(false).unwrap();
        assert_eq!(convs.len(), 1);
        let msgs = store.messages(convs[0].id).unwrap();
        assert_eq!(msgs[1].sources[0].locator.as_deref(), Some("p.3"));

        // Running again merges instead of duplicating.
        let again = run(&home, &store, &dump, &root, &ImportOptions::default()).unwrap();
        assert_eq!(again.files_imported, 0);
        assert_eq!(again.files_merged, 2);
        assert_eq!(
            store
                .files_query(&georag_store::FileQuery::default())
                .unwrap()
                .len(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_reads_like_the_database() {
        let dir = std::env::temp_dir().join(format!("georag-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("backup.georag");
        {
            use std::io::Write;
            let f = std::fs::File::create(&path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("manifest.json", o).unwrap();
            z.write_all(br#"{"format_version": 1}"#).unwrap();
            z.start_file("files.jsonl", o).unwrap();
            z.write_all(
                b"{\"id\": 1, \"relative_path\": \"x/y.pdf\", \"scan_status\": \"processed\"}\n",
            )
            .unwrap();
            z.start_file("tags.json", o).unwrap();
            z.write_all(br#"[{"id": 1, "name": "piling", "display_name": "Piling"}]"#)
                .unwrap();
            z.start_file("file_tags.jsonl", o).unwrap();
            z.write_all(
                b"{\"file_id\": 1, \"tag_id\": 1, \"source\": \"auto\", \"confidence\": 0.8}\n",
            )
            .unwrap();
            z.start_file("vectors/tag_piling.npz", o).unwrap();
            z.write_all(b"binary").unwrap();
            z.finish().unwrap();
        }
        let (located, dump) = locate(&path).unwrap();
        assert_eq!(located.kind, InputKind::Archive);
        assert_eq!(dump.files[0].filename, "y.pdf");
        assert_eq!(dump.file_tags.len(), 1);
        assert_eq!(map_source(dump.file_tags[0].source.as_deref()), "llm");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
