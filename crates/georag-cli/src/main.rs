//! `georag`: the single binary. No arguments opens the TUI; subcommands run
//! the same core in-process (and through the daemon once P3 lands).
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use georag_core::doctor::{self, DoctorOptions};
use georag_core::{Home, logging};

#[derive(Parser, Debug)]
#[command(
    name = "georag",
    version = georag_core::VERSION,
    about = "Local assistant for geotechnical engineers: indexes a document library and answers questions from it, all on this machine"
)]
struct Cli {
    /// Home folder. Defaults to GEORAG_HOME, then the folder holding the executable.
    #[arg(long, global = true, env = "GEORAG_HOME", value_name = "DIR")]
    home: Option<PathBuf>,

    /// Verify the engine after an update and exit (used by the updater)
    #[arg(long, hide = true)]
    self_check: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create the home folder layout and default settings
    Init,
    /// Show the home folder, engine, models, library and jobs at a glance
    Status {
        /// Emit the snapshot as JSON
        #[arg(long)]
        json: bool,
    },
    /// Report the build flavour, backend, device memory and model fit
    Doctor {
        /// Print the cosine score distribution for this sample question
        #[arg(long, value_name = "QUESTION")]
        scores: Option<String>,
        /// CI mode: force the CPU backend and skip anything that needs a model
        #[arg(long)]
        ci: bool,
        /// Override settings.engine.backend (auto | metal | cuda | vulkan | cpu)
        #[arg(long, value_name = "NAME")]
        backend: Option<String>,
        /// Emit the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage chat, embedding and vision models
    Models {
        #[command(subcommand)]
        cmd: ModelsCmd,
    },
    /// Ask one question and print the answer with its sources
    Ask {
        question: String,
        /// Restrict to files with these tags (comma separated)
        #[arg(short = 't', long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Continue an existing conversation
        #[arg(short = 'c', long, value_name = "ID")]
        conversation: Option<i64>,
        /// auto | manual
        #[arg(long)]
        mode: Option<String>,
        /// Manual tier: quick | optimal | deep | deeper | ludicrous
        #[arg(long, value_name = "TIER")]
        depth: Option<String>,
        #[arg(long)]
        min_relevance: Option<f32>,
        #[arg(long)]
        max_passes: Option<usize>,
        #[arg(long)]
        no_synthesis: bool,
        #[arg(long)]
        json: bool,
    },
    /// Interactive chat in the terminal (one line per question, empty line quits)
    Chat {
        #[arg(short = 't', long, value_delimiter = ',')]
        tags: Vec<String>,
        #[arg(short = 'c', long, value_name = "ID")]
        conversation: Option<i64>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long, value_name = "TIER")]
        depth: Option<String>,
    },
    /// Hybrid search over the index without asking the model
    Search {
        query: String,
        #[arg(short = 't', long, value_delimiter = ',')]
        tags: Vec<String>,
        #[arg(long, default_value = "10")]
        k: usize,
        #[arg(long)]
        json: bool,
    },
    /// List, show, delete, restore or export conversations
    Conversations {
        #[command(subcommand)]
        cmd: ConversationsCmd,
    },
    /// Check for, download and apply a release
    Update {
        /// Only report whether a newer release exists
        #[arg(long)]
        check: bool,
        /// stable | nightly (default from settings)
        #[arg(long)]
        channel: Option<String>,
        /// Apply a bundle from disk instead of downloading
        #[arg(long, value_name = "BUNDLE")]
        file: Option<PathBuf>,
        /// Put the previous version back
        #[arg(long)]
        rollback: bool,
        /// Do not ask before applying
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Read or change settings.toml keys
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Library roots: folders holding the documents
    Library {
        #[command(subcommand)]
        cmd: LibraryCmd,
    },
    /// Walk the library roots and queue changed files
    Scan,
    /// Run the pipeline: extract, tag, chunk, embed, index
    Index {
        /// Keep watching the roots and index changes as they happen
        #[arg(long)]
        watch: bool,
        /// Only files with these tags (comma separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Re-extract files that are already indexed
        #[arg(long)]
        reprocess: bool,
        /// Only these file ids
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// Rules only, no LLM tagging
        #[arg(long)]
        no_llm: bool,
    },
    /// Indexed documents
    Docs {
        #[command(subcommand)]
        cmd: DocsCmd,
    },
    /// Topic tags
    Tags {
        #[command(subcommand)]
        cmd: TagsCmd,
    },
    /// Import tags, the file list and optionally chats from the v1 app
    Import {
        #[command(subcommand)]
        cmd: ImportCmd,
    },
    /// Development helpers
    #[command(hide = true)]
    Dev {
        #[command(subcommand)]
        cmd: DevCmd,
    },
}

#[derive(Subcommand, Debug)]
enum ConversationsCmd {
    List {
        #[arg(long)]
        json: bool,
    },
    Show {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    Rm {
        id: i64,
    },
    Restore {
        id: i64,
    },
    /// Write the conversation as markdown into exports/
    Export {
        id: i64,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// Print one key, or every key when none is given
    Get {
        key: Option<String>,
    },
    Set {
        key: String,
        value: String,
    },
}

#[derive(Subcommand, Debug)]
enum LibraryCmd {
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    /// Re-point a root after a drive letter or mount changed
    Move {
        name: String,
        path: PathBuf,
    },
    Remove {
        name: String,
    },
    /// Forget files that are missing on disk
    Prune,
}

#[derive(Subcommand, Debug)]
enum DocsCmd {
    List {
        #[arg(long)]
        q: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        ext: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value = "200")]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    Show {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    Tag {
        id: i64,
        tag: String,
    },
    Untag {
        id: i64,
        tag: String,
    },
    /// Open the file with the operating system
    Open {
        id: i64,
    },
}

#[derive(Subcommand, Debug)]
enum TagsCmd {
    List {
        #[arg(long)]
        json: bool,
    },
    Add {
        name: String,
        #[arg(long)]
        display: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        color: Option<String>,
        /// Folder and file name patterns (comma separated)
        #[arg(long, value_delimiter = ',')]
        patterns: Vec<String>,
    },
    Edit {
        name: String,
        #[arg(long)]
        display: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long, value_delimiter = ',')]
        patterns: Option<Vec<String>>,
    },
    Remove {
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum ImportCmd {
    /// From a v1 _0RAG folder, its data/metadata.db, or a .georag archive
    V1 {
        path: PathBuf,
        /// Folder the v1 relative paths are based on (default: the parent of _0RAG)
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long)]
        with_chats: bool,
        /// Run the tag stage again instead of keeping imported tags
        #[arg(long)]
        retag: bool,
        /// Preview only
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum DevCmd {
    /// Seed a synthetic geotechnical corpus into the index
    Seed {
        #[arg(long, default_value = "8")]
        sites: usize,
        #[arg(long, default_value = "5")]
        chunks: usize,
    },
}

#[derive(Subcommand, Debug)]
enum ModelsCmd {
    /// Installed models and whether they are present on disk
    List {
        #[arg(long)]
        json: bool,
    },
    /// The curated catalogue, optionally filtered
    Catalog {
        #[arg(value_name = "QUERY")]
        query: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Download from Hugging Face: a catalogue id, or ORG/REPO[:FILE]
    Pull {
        spec: String,
        /// File within the repo (alternative to ORG/REPO:FILE)
        file: Option<String>,
        #[arg(long, value_name = "ROLE")]
        role: Option<String>,
        /// Mark the model as the one in use for its role once pulled
        #[arg(long)]
        r#use: bool,
    },
    /// Register a GGUF that already exists on disk
    Link {
        path: PathBuf,
        #[arg(long, value_name = "ROLE")]
        role: String,
        #[arg(long)]
        id: Option<String>,
    },
    /// Choose the model in use for a role
    Use {
        #[arg(value_name = "ROLE")]
        role: String,
        id: String,
    },
    /// Forget a model; managed files are deleted
    Remove { id: String },
    /// Hash the file and compare with the recorded sha256
    Verify { id: String },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let home = Home::resolve(cli.home.as_deref())?;
    if cli.self_check {
        let settings =
            georag_core::config::Settings::load(&home.settings_path()).unwrap_or_default();
        let report = doctor::run(
            &home,
            &settings,
            &DoctorOptions {
                backend: None,
                ci: true,
                scores: None,
            },
        )?;
        println!(
            "self-check ok: {} {} on {}",
            report.version, report.flavour, report.backend.backend
        );
        return Ok(());
    }

    match cli.command {
        Some(Command::Init) => {
            let _logs = logging::init(&home, true).ok();
            let report = home.init()?;
            println!("home folder: {}", home.root().display());
            for dir in &report.created_dirs {
                println!("  created  {}", dir.display());
            }
            for file in &report.created_files {
                println!("  wrote    {}", file.display());
            }
            if report.created_dirs.is_empty() && report.created_files.is_empty() {
                println!("  nothing to do, the layout was already in place");
            }
            Ok(())
        }
        Some(Command::Doctor {
            scores,
            ci,
            backend,
            json,
        }) => {
            let _logs = logging::init(&home, !json).ok();
            let settings = georag_core::config::Settings::load(&home.settings_path())?;
            let report = doctor::run(
                &home,
                &settings,
                &DoctorOptions {
                    backend,
                    ci,
                    scores,
                },
            )?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).context("serialising the report")?
                );
            } else {
                print!("{report}");
            }
            Ok(())
        }
        Some(Command::Status { json }) => {
            let _logs = logging::init(&home, !json).ok();
            let settings = georag_core::config::Settings::load(&home.settings_path())?;
            let snapshot = georag_core::status::Snapshot::take(&home, &settings);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot).context("serialising the snapshot")?
                );
            } else {
                print!("{snapshot}");
            }
            Ok(())
        }
        Some(Command::Models { cmd }) => {
            let _logs = logging::init(&home, true).ok();
            models_cmd(&home, cmd)
        }
        Some(Command::Ask {
            question,
            tags,
            conversation,
            mode,
            depth,
            min_relevance,
            max_passes,
            no_synthesis,
            json,
        }) => {
            let _logs = logging::init(&home, !json).ok();
            let req = chat_request(
                question,
                tags,
                conversation,
                mode,
                depth,
                min_relevance,
                max_passes,
                no_synthesis,
            )?;
            let app = open_app(&home, &req)?;
            ask_once(&app, &req, json).map(|_| ())
        }
        Some(Command::Chat {
            tags,
            conversation,
            mode,
            depth,
        }) => {
            let _logs = logging::init(&home, true).ok();
            let mut req = chat_request(
                String::new(),
                tags,
                conversation,
                mode,
                depth,
                None,
                None,
                false,
            )?;
            let app = open_app(&home, &req)?;
            println!("georag chat: type a question and press enter; an empty line quits.");
            loop {
                use std::io::Write;
                print!("\n> ");
                std::io::stdout().flush().ok();
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line)? == 0 || line.trim().is_empty() {
                    break;
                }
                req.message = line.trim().to_string();
                match ask_once(&app, &req, false) {
                    Ok(outcome) => req.conversation_id = Some(outcome.conversation_id),
                    Err(e) => eprintln!("error: {e:#}"),
                }
            }
            Ok(())
        }
        Some(Command::Search {
            query,
            tags,
            k,
            json,
        }) => {
            let _logs = logging::init(&home, !json).ok();
            let app = georag_core::app::App::open(&home, None)?;
            let results = app.search(&query, &tags, k)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for r in results {
                    println!(
                        "{:.3}  {} {}  #{}\n       {}",
                        r.score,
                        r.rel_path,
                        r.locator.as_deref().unwrap_or(""),
                        r.chunk_id,
                        r.snippet
                    );
                }
            }
            Ok(())
        }
        Some(Command::Conversations { cmd }) => {
            let _logs = logging::init(&home, true).ok();
            conversations_cmd(&home, cmd)
        }
        Some(Command::Update {
            check,
            channel,
            file,
            rollback,
            yes,
            json,
        }) => {
            let _logs = logging::init(&home, !json).ok();
            update_cmd(&home, check, channel, file, rollback, yes, json)
        }
        Some(Command::Config { cmd }) => {
            let mut settings = georag_core::config::Settings::load(&home.settings_path())?;
            match cmd {
                ConfigCmd::Get { key: Some(k) } => match settings.get(&k) {
                    Some(v) => println!("{v}"),
                    None => anyhow::bail!("unknown setting {k:?}"),
                },
                ConfigCmd::Get { key: None } => {
                    for (k, v) in settings.entries() {
                        println!("{k} = {v}");
                    }
                }
                ConfigCmd::Set { key, value } => {
                    settings.set(&key, &value)?;
                    settings.save(&home.settings_path())?;
                    println!("{key} = {value}");
                }
            }
            Ok(())
        }
        Some(Command::Library { cmd }) => {
            let _logs = logging::init(&home, true).ok();
            library_cmd(&home, cmd)
        }
        Some(Command::Scan) => {
            let _logs = logging::init(&home, true).ok();
            let store = georag_core::app::open_store(&home)?;
            for r in georag_core::pipeline::scan_all(&store)? {
                println!(
                    "{:<16} {} seen, {} queued, {} unchanged, {} missing",
                    r.root, r.seen, r.queued, r.unchanged, r.missing
                );
            }
            Ok(())
        }
        Some(Command::Index {
            watch,
            tags,
            reprocess,
            ids,
            no_llm,
        }) => {
            let _logs = logging::init(&home, true).ok();
            index_cmd(&home, watch, tags, reprocess, ids, no_llm)
        }
        Some(Command::Docs { cmd }) => {
            let _logs = logging::init(&home, true).ok();
            docs_cmd(&home, cmd)
        }
        Some(Command::Tags { cmd }) => {
            let _logs = logging::init(&home, true).ok();
            tags_cmd(&home, cmd)
        }
        Some(Command::Import {
            cmd:
                ImportCmd::V1 {
                    path,
                    root,
                    with_chats,
                    retag,
                    dry_run,
                    json,
                },
        }) => {
            let _logs = logging::init(&home, !json).ok();
            import_v1_cmd(&home, path, root, with_chats, retag, dry_run, json)
        }
        Some(Command::Dev { cmd }) => {
            let _logs = logging::init(&home, true).ok();
            match cmd {
                DevCmd::Seed { sites, chunks } => {
                    let app = georag_core::app::App::open(&home, None)?;
                    let tokens = app.seed_demo(sites, chunks)?;
                    println!(
                        "seeded {sites} documents, {} chunks, {tokens} chat tokens",
                        sites * chunks
                    );
                    Ok(())
                }
            }
        }
        None => {
            // First launch from a fresh folder: create the layout so the UI
            // has settings and a log. Everything stays inside the home folder.
            if !home.is_initialised() {
                home.init()?;
            }
            let _logs = logging::init(&home, false).ok();
            let settings = georag_core::config::Settings::load(&home.settings_path())?;
            georag_tui::run(&home, &settings)?;
            Ok(())
        }
    }
}

fn models_cmd(home: &Home, cmd: ModelsCmd) -> anyhow::Result<()> {
    use georag_core::models::{ModelRole, Registry, catalog, ops};
    let mut registry = Registry::load(home)?;
    match cmd {
        ModelsCmd::List { json } => {
            let views = ops::list(home, &registry, None);
            if json {
                println!("{}", serde_json::to_string_pretty(&views)?);
                return Ok(());
            }
            if views.is_empty() {
                println!("no models yet: try `georag models catalog` then `georag models pull ID`");
                return Ok(());
            }
            println!(
                "{:<8} {:<28} {:<8} {:>9} {:>8}  where",
                "role", "id", "kind", "size", "n_ctx"
            );
            for v in views {
                let size = v
                    .entry
                    .size_bytes
                    .map(georag_core::doctor::fmt_gib)
                    .unwrap_or_else(|| "?".into());
                let ctx = v
                    .entry
                    .n_ctx_train
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                let flag = if v.entry.in_use { "*" } else { " " };
                let state = if v.present { "" } else { "  (missing on disk)" };
                println!(
                    "{flag}{:<7} {:<28} {:<8} {:>9} {:>8}  {}{state}",
                    v.entry.role.as_str(),
                    v.entry.id,
                    match v.entry.kind {
                        georag_core::models::ModelKind::Managed => "managed",
                        georag_core::models::ModelKind::Linked => "linked",
                    },
                    size,
                    ctx,
                    v.resolved_path.display()
                );
            }
            println!("* = in use for its role");
            Ok(())
        }
        ModelsCmd::Catalog { query, json } => {
            let entries = catalog::search(query.as_deref().unwrap_or(""));
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
                return Ok(());
            }
            println!("{:<8} {:<26} {:>9}  repo and notes", "role", "id", "size");
            for e in entries {
                println!(
                    "{:<8} {:<26} {:>9}  {}:{}\n{:>46}  {}",
                    e.role.as_str(),
                    e.id,
                    georag_core::doctor::fmt_gib(e.size_bytes),
                    e.repo,
                    e.file,
                    "",
                    e.notes
                );
            }
            Ok(())
        }
        ModelsCmd::Pull {
            spec,
            file,
            role,
            r#use,
        } => {
            let spec = match file {
                Some(f) => format!("{spec}:{f}"),
                None => spec,
            };
            let role = role.map(|r| r.parse::<ModelRole>()).transpose()?;
            let mut plan = ops::plan_pull(home, &spec, role)?;
            ops::complete_plan(&mut plan)?;
            println!(
                "pulling {}:{} as {} ({})",
                plan.repo, plan.file, plan.id, plan.role
            );
            let pulls = georag_core::models::PullManager::new();
            ops::start_pull(&pulls, &plan)?;
            let bar = indicatif::ProgressBar::new(plan.size.unwrap_or(0));
            bar.set_style(
                indicatif::ProgressStyle::with_template(
                    "{bar:30} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
                )
                .expect("static template"),
            );
            let status = pulls.wait(&plan.id, |st| {
                if let Some(t) = st.total {
                    bar.set_length(t);
                }
                bar.set_position(st.bytes);
            })?;
            bar.finish_and_clear();
            if status.state != georag_core::models::PullState::Done {
                anyhow::bail!("pull did not finish: {:?}", status.state);
            }
            let entry = ops::register_pulled(home, &mut registry, &plan)?;
            if r#use {
                ops::use_model(home, &mut registry, &entry.id)?;
            }
            let in_use = registry.get(&entry.id).map(|m| m.in_use).unwrap_or(false);
            println!(
                "done: {} at {}{}",
                entry.id,
                entry.resolve_path(home).display(),
                if in_use { " (in use)" } else { "" }
            );
            Ok(())
        }
        ModelsCmd::Link { path, role, id } => {
            let entry = ops::link(home, &mut registry, &path, role.parse()?, id.as_deref())?;
            println!("linked {} ({}) from {}", entry.id, entry.role, entry.path);
            Ok(())
        }
        ModelsCmd::Use { role, id } => {
            let role: ModelRole = role.parse()?;
            let entry = ops::use_model(home, &mut registry, &id)?;
            if entry.role != role {
                anyhow::bail!("{} is a {} model, not {}", id, entry.role, role);
            }
            println!("{} is now the {} model in use", entry.id, entry.role);
            Ok(())
        }
        ModelsCmd::Remove { id } => {
            let entry = ops::remove(home, &mut registry, &id)?;
            println!("removed {}", entry.id);
            Ok(())
        }
        ModelsCmd::Verify { id } => {
            let report = ops::verify(home, &registry, &id)?;
            match report.ok {
                Some(true) => println!("ok: {} matches {}", report.id, report.actual),
                Some(false) => anyhow::bail!(
                    "{} is corrupt: expected {:?}, got {}",
                    report.id,
                    report.expected,
                    report.actual
                ),
                None => println!(
                    "{} hashes to {} (no recorded sha256 to compare)",
                    report.id, report.actual
                ),
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn chat_request(
    message: String,
    tags: Vec<String>,
    conversation: Option<i64>,
    mode: Option<String>,
    depth: Option<String>,
    min_relevance: Option<f32>,
    max_passes: Option<usize>,
    no_synthesis: bool,
) -> anyhow::Result<georag_core::chat::ChatRequest> {
    use georag_core::chat::{Mode, Tier};
    let tier = depth.as_deref().map(str::parse::<Tier>).transpose()?;
    let mode = match (mode.as_deref().map(str::parse::<Mode>).transpose()?, tier) {
        (Some(m), _) => Some(m),
        (None, Some(_)) => Some(Mode::Manual),
        (None, None) => None,
    };
    Ok(georag_core::chat::ChatRequest {
        conversation_id: conversation,
        message,
        tags: tags
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        mode,
        tier,
        min_relevance,
        max_passes,
        synthesis: if no_synthesis { Some(false) } else { None },
    })
}

fn open_app(
    home: &Home,
    req: &georag_core::chat::ChatRequest,
) -> anyhow::Result<georag_core::app::App> {
    use georag_core::chat::Mode;
    let want = match (req.mode, req.tier) {
        (Some(Mode::Manual), Some(t)) => Some(t.n_ctx()),
        _ => None,
    };
    Ok(georag_core::app::App::open(home, want)?)
}

fn ask_once(
    app: &georag_core::app::App,
    req: &georag_core::chat::ChatRequest,
    json: bool,
) -> anyhow::Result<georag_core::chat::RunOutcome> {
    use georag_core::chat::{ChatEvent, Controls};
    use std::io::Write;
    let ctx = app.chat_context();
    let controls = Controls::new();
    let mut out = std::io::stdout();
    let mut current_pass: Option<u16> = None;
    let outcome = georag_core::chat::run(&ctx, req, &controls, &mut |ev| {
        if json {
            return;
        }
        match ev {
            ChatEvent::Plan { summary } => {
                if summary.passes > 1 {
                    println!(
                        "plan: {} passes over {} sources ({} tokens, budget {} per pass{}{})",
                        summary.passes,
                        summary.kept,
                        summary.total_tokens,
                        summary.budget,
                        if summary.dropped > 0 {
                            format!(", {} dropped", summary.dropped)
                        } else {
                            String::new()
                        },
                        if summary.fallback {
                            ", nothing above the threshold"
                        } else {
                            ""
                        }
                    );
                } else {
                    println!(
                        "sources: {} ({} tokens){}",
                        summary.kept,
                        summary.total_tokens,
                        if summary.fallback {
                            ", nothing above the threshold"
                        } else {
                            ""
                        }
                    );
                }
            }
            ChatEvent::PassStart {
                k,
                sources_from,
                sources_to,
            } => {
                current_pass = Some(k);
                println!("\n--- pass {k} (sources {sources_from} to {sources_to}) ---");
            }
            ChatEvent::Token { k, text } => {
                if k == current_pass || k.is_none() || k == Some(0) {
                    print!("{text}");
                    out.flush().ok();
                }
            }
            ChatEvent::PassEnd { k, status, cut_off } => {
                println!(
                    "\n--- pass {k} {}{} ---",
                    status.as_str(),
                    if cut_off { ", cut off" } else { "" }
                );
            }
            ChatEvent::FinalStart => {
                current_pass = Some(0);
                println!("\n--- combining ---");
            }
            ChatEvent::Done {
                sources, cut_off, ..
            } => {
                println!();
                if !sources.is_empty() {
                    println!("\nsources:");
                    for s in sources {
                        println!(
                            "  [S{}] {} {}  (score {:.2})",
                            s.tag,
                            s.rel_path,
                            s.locator.as_deref().unwrap_or(""),
                            s.score
                        );
                    }
                }
                if cut_off {
                    println!("note: the answer was cut off at the context limit");
                }
            }
            ChatEvent::Error { message } => eprintln!("error: {message}"),
        }
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    }
    Ok(outcome)
}

fn conversations_cmd(home: &Home, cmd: ConversationsCmd) -> anyhow::Result<()> {
    use georag_core::conversations as conv;
    match cmd {
        ConversationsCmd::List { json } => {
            let convs = conv::list(home, false)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&convs)?);
            } else if convs.is_empty() {
                println!("no conversations yet");
            } else {
                for c in convs {
                    println!(
                        "{:>5}  {}  {}",
                        c.id,
                        c.updated_at,
                        c.title.as_deref().unwrap_or("(untitled)")
                    );
                }
            }
        }
        ConversationsCmd::Show { id, json } => {
            let view = conv::show(home, id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                println!(
                    "{} ({})",
                    view.conversation.title.as_deref().unwrap_or("(untitled)"),
                    view.conversation.created_at
                );
                for m in view.messages {
                    println!("\n[{}] {}", m.message.role, m.message.content);
                    if m.passes.len() > 1 {
                        println!(
                            "  passes: {}",
                            m.passes
                                .iter()
                                .map(|p| format!("{}={}", p.idx, p.status))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                    }
                }
            }
        }
        ConversationsCmd::Rm { id } => {
            conv::delete(home, id)?;
            println!(
                "conversation {id} moved to trash (restore with `georag conversations restore {id}`)"
            );
        }
        ConversationsCmd::Restore { id } => {
            conv::restore(home, id)?;
            println!("conversation {id} restored");
        }
        ConversationsCmd::Export { id } => {
            let path = conv::export(home, id)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

fn update_cmd(
    home: &Home,
    check_only: bool,
    channel: Option<String>,
    file: Option<PathBuf>,
    rollback: bool,
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    use georag_core::update;
    let current_exe = std::env::current_exe()?;
    if rollback {
        let done = update::rollback(home, &current_exe)?;
        println!(
            "{}",
            if done {
                "previous version restored"
            } else {
                "nothing to roll back: no .old copies found"
            }
        );
        return Ok(());
    }
    let mut settings = georag_core::config::Settings::load(&home.settings_path())?;
    let channel = channel.unwrap_or_else(|| settings.update.channel.clone());

    let offline = file.is_some();
    let staged = if let Some(bundle) = file {
        let sha = update::verify_bundle(&bundle, None)?;
        println!("bundle {} ({sha})", bundle.display());
        update::stage(home, &bundle)?
    } else {
        let report = update::check(&channel)?;
        if json && check_only {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        println!(
            "current {}  latest {} ({})",
            report.current, report.latest, report.channel
        );
        for n in &report.notes {
            println!("note: {n}");
        }
        if !report.newer {
            println!("up to date");
            return Ok(());
        }
        if !report.can_update_directly {
            anyhow::bail!(
                "this version is too old to update directly: install the latest release by hand"
            );
        }
        let asset = report
            .asset
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no asset for this flavour"))?;
        println!(
            "newer release {} available: {} ({})",
            report.latest,
            asset.file,
            georag_core::doctor::fmt_gib(asset.size)
        );
        if check_only {
            let _ = update::check_for_notice(home, &mut settings);
            return Ok(());
        }
        if !yes {
            use std::io::Write;
            print!("download and apply? [y/N] ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if !line.trim().eq_ignore_ascii_case("y") {
                println!("not updating");
                return Ok(());
            }
        }
        let bar = indicatif::ProgressBar::new(asset.size);
        bar.set_style(
            indicatif::ProgressStyle::with_template(
                "{bar:30} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
            )
            .expect("static template"),
        );
        let dl = update::download(home, &report, &channel, |bytes, total| {
            if let Some(t) = total {
                bar.set_length(t);
            }
            bar.set_position(bytes);
        })?;
        bar.finish_and_clear();
        println!("downloaded and verified {}", dl.bundle.display());
        update::stage(home, &dl.bundle)?
    };
    println!("staged at {}", staged.display());
    if !yes && offline {
        use std::io::Write;
        print!("apply? [y/N] ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("not applied; staged files stay in data/updates/staging");
            return Ok(());
        }
    }
    let report = update::apply(home, &staged, &current_exe)?;
    if report.rolled_back {
        anyhow::bail!(
            "the new version failed its self-check and the previous version was restored"
        );
    }
    println!("updated: run `georag doctor` to confirm");
    Ok(())
}

fn library_cmd(home: &Home, cmd: LibraryCmd) -> anyhow::Result<()> {
    use georag_core::library;
    let store = georag_core::app::open_store(home)?;
    match cmd {
        LibraryCmd::Add { path, name } => {
            let root = library::add(&store, &path, name.as_deref())?;
            let report = georag_core::pipeline::scan_root(&store, &root)?;
            println!(
                "added root {:?} at {}: {} files seen, {} queued. Run `georag index` to index them.",
                root.name, root.path, report.seen, report.queued
            );
        }
        LibraryCmd::List { json } => {
            let roots = library::list(&store)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&roots)?);
            } else if roots.is_empty() {
                println!("no roots yet: `georag library add PATH`");
            } else {
                for r in roots {
                    println!("{:<16} {}", r.name, r.path);
                }
            }
        }
        LibraryCmd::Move { name, path } => {
            library::move_root(&store, &name, &path)?;
            println!("root {name:?} now points at {}", path.display());
        }
        LibraryCmd::Remove { name } => {
            library::remove(&store, &name)?;
            println!("removed root {name:?} and its index entries");
        }
        LibraryCmd::Prune => {
            let n = library::prune(&store)?;
            println!("pruned {n} missing files");
        }
    }
    Ok(())
}

fn index_cmd(
    home: &Home,
    watch: bool,
    tags: Vec<String>,
    reprocess: bool,
    ids: Vec<i64>,
    no_llm: bool,
) -> anyhow::Result<()> {
    use georag_core::pipeline::{self, IndexOptions};
    let app = georag_core::app::App::open(home, None)?;
    let opts = IndexOptions {
        tags,
        reprocess,
        ids,
        no_llm,
    };
    let mut on_event = |ev: pipeline::PipelineEvent| {
        println!(
            "[{}/{}] {:<8} {}  {}",
            ev.done, ev.total, ev.stage, ev.rel_path, ev.outcome
        );
    };
    let scans = pipeline::scan_all(&app.store)?;
    for r in &scans {
        println!(
            "scan {:<12} {} seen, {} queued, {} missing",
            r.root, r.seen, r.queued, r.missing
        );
    }
    pipeline::supervisor::queue(&app, &opts)?;
    let report = pipeline::run(&app, &opts, &mut on_event, &|| true)?;
    println!(
        "extracted {} · tagged {} · indexed {} · failed {} · skipped {}",
        report.extracted, report.tagged, report.embedded, report.failed, report.skipped
    );
    if !watch {
        return Ok(());
    }
    let roots: Vec<PathBuf> = app
        .store
        .roots()?
        .into_iter()
        .map(|r| PathBuf::from(r.path))
        .collect();
    let debounce = std::time::Duration::from_millis(app.settings.index.watch_debounce_ms);
    let watcher = pipeline::watch::Watcher::new(&roots, debounce)?;
    println!("watching {} root(s); ctrl-c stops", roots.len());
    loop {
        let changed = watcher.changed_roots(std::time::Duration::from_secs(3600));
        if changed.is_empty() {
            continue;
        }
        for root in app.store.roots()? {
            if changed
                .iter()
                .any(|c| c == std::path::Path::new(&root.path))
            {
                match pipeline::scan_root(&app.store, &root) {
                    Ok(r) => println!(
                        "scan {:<12} {} seen, {} queued, {} missing",
                        r.root, r.seen, r.queued, r.missing
                    ),
                    Err(e) => eprintln!("scan failed: {e}"),
                }
            }
        }
        let report = pipeline::run(&app, &IndexOptions::default(), &mut on_event, &|| true)?;
        if report.extracted + report.embedded + report.failed > 0 {
            println!("indexed {} · failed {}", report.embedded, report.failed);
        }
    }
}

fn docs_cmd(home: &Home, cmd: DocsCmd) -> anyhow::Result<()> {
    let store = georag_core::app::open_store(home)?;
    match cmd {
        DocsCmd::List {
            q,
            state,
            ext,
            tag,
            limit,
            json,
        } => {
            let state = state
                .map(|s| {
                    georag_core::FileState::parse(&s)
                        .ok_or_else(|| anyhow::anyhow!("unknown state {s:?}"))
                })
                .transpose()?;
            let query = georag_core::FileQuery {
                text: q,
                state,
                ext,
                tag,
                root_id: None,
                offset: 0,
                limit,
            };
            let files = store.files_query(&query)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&files)?);
            } else {
                let facets = store.files_facets(&query)?;
                for f in &files {
                    let tags: Vec<String> =
                        store.file_tags(f.id)?.into_iter().map(|t| t.tag).collect();
                    println!(
                        "{:>6}  {:<9} {:>5}  {}  [{}]",
                        f.id,
                        f.state.as_str(),
                        f.chunk_count,
                        f.rel_path,
                        tags.join(",")
                    );
                }
                println!("{} of {} files", files.len(), facets.total);
            }
        }
        DocsCmd::Show { id, json } => {
            let file = store
                .file_get(id)?
                .ok_or_else(|| anyhow::anyhow!("no file {id}"))?;
            let tags = store.file_tags(id)?;
            let chunks = store.chunks_for_file(id)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "file": file, "tags": tags, "chunks": chunks.len() })
                    )?
                );
            } else {
                println!("{}  ({})", file.rel_path, file.state.as_str());
                println!(
                    "  chunks {}  tokens {}  {}",
                    file.chunk_count,
                    file.token_count,
                    file.error
                        .as_deref()
                        .map(|e| format!("error: {e}"))
                        .unwrap_or_default()
                );
                for t in tags {
                    println!(
                        "  tag {} ({}{})",
                        t.tag,
                        t.source,
                        t.confidence.map(|c| format!(" {c:.1}")).unwrap_or_default()
                    );
                }
                if let Some(p) = &file.preview {
                    println!("  {}", p.replace('\n', " "));
                }
                for c in chunks.iter().take(5) {
                    println!(
                        "  #{} {} {} tokens",
                        c.id,
                        c.locator.as_deref().unwrap_or(""),
                        c.tokens
                    );
                }
            }
        }
        DocsCmd::Tag { id, tag } => {
            store.file_tag_add(id, &tag, "manual", None, false)?;
            println!("tagged {id} with {tag}");
        }
        DocsCmd::Untag { id, tag } => {
            store.file_tag_remove(id, &tag)?;
            println!("removed {tag} from {id}");
        }
        DocsCmd::Open { id } => {
            let file = store
                .file_get(id)?
                .ok_or_else(|| anyhow::anyhow!("no file {id}"))?;
            let roots = store.roots()?;
            let root = roots
                .iter()
                .find(|r| r.id == file.root_id)
                .ok_or_else(|| anyhow::anyhow!("root missing"))?;
            let path = georag_core::library::absolute_path(root, &file.rel_path);
            let program = if cfg!(target_os = "macos") {
                "open"
            } else if cfg!(target_os = "windows") {
                "cmd"
            } else {
                "xdg-open"
            };
            let mut c = std::process::Command::new(program);
            if cfg!(target_os = "windows") {
                c.args(["/C", "start", ""]);
            }
            c.arg(&path).spawn()?;
            println!("opened {}", path.display());
        }
    }
    Ok(())
}

fn tags_cmd(home: &Home, cmd: TagsCmd) -> anyhow::Result<()> {
    use georag_core::tags::{TagDef, TagFile};
    let mut defs = TagFile::load_or_create(home)?;
    let store = georag_core::app::open_store(home)?;
    match cmd {
        TagsCmd::List { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&defs.tags)?);
            } else {
                let counts = store.tag_counts()?;
                for t in &defs.tags {
                    let n = counts
                        .iter()
                        .find(|(name, _)| name == &t.name)
                        .map(|(_, n)| *n)
                        .unwrap_or(0);
                    println!("{:<28} {:>5}  {}", t.name, n, t.display_name);
                }
            }
            return Ok(());
        }
        TagsCmd::Add {
            name,
            display,
            description,
            color,
            patterns,
        } => {
            defs.upsert(TagDef {
                name: name.clone(),
                display_name: display.unwrap_or_else(|| name.clone()),
                color,
                description,
                patterns,
            });
            println!("added tag {name}");
        }
        TagsCmd::Edit {
            name,
            display,
            description,
            color,
            patterns,
        } => {
            let mut t = defs
                .get(&name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no tag {name:?}"))?;
            if let Some(d) = display {
                t.display_name = d;
            }
            if description.is_some() {
                t.description = description;
            }
            if color.is_some() {
                t.color = color;
            }
            if let Some(p) = patterns {
                t.patterns = p;
            }
            defs.upsert(t);
            println!("updated tag {name}");
        }
        TagsCmd::Remove { name } => {
            if !defs.remove(&name) {
                anyhow::bail!("no tag {name:?}");
            }
            store.tag_remove(&name)?;
            println!("removed tag {name}");
        }
    }
    defs.save(home)?;
    defs.seed(&store)?;
    Ok(())
}

fn import_v1_cmd(
    home: &Home,
    path: PathBuf,
    root: Option<PathBuf>,
    with_chats: bool,
    retag: bool,
    dry_run: bool,
    json: bool,
) -> anyhow::Result<()> {
    use georag_core::import_v1 as imp;
    let (located, dump) = imp::locate(&path)?;
    let root = match root.or(located.default_root.clone()) {
        Some(r) => r,
        None => anyhow::bail!("the archive does not say where the documents live: pass --root DIR"),
    };
    let store = georag_core::app::open_store(home)?;
    let defs = georag_core::tags::TagFile::load_or_create(home)?;
    let preview = imp::preview(&store, &dump, &root, &defs)?;
    if json && dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "located": located, "preview": preview })
            )?
        );
        return Ok(());
    }
    if !json {
        println!("source   {} ({:?})", located.source.display(), located.kind);
        println!("root     {}", root.display());
        println!(
            "files    {} ({} on disk, {} missing, {} failed in v1, {} already present)",
            preview.files,
            preview.files_on_disk,
            preview.files_missing,
            preview.files_failed_in_v1,
            preview.already_present
        );
        println!(
            "tags     {} ({} built in, {} custom{})",
            preview.tags,
            preview.tags_builtin,
            preview.tags_custom.len(),
            if preview.tags_custom.is_empty() {
                String::new()
            } else {
                format!(": {}", preview.tags_custom.join(", "))
            }
        );
        println!("file tags {}", preview.file_tags);
        println!(
            "chats    {} conversations, {} messages{}",
            preview.conversations,
            preview.messages,
            if with_chats {
                ""
            } else {
                " (not imported without --with-chats)"
            }
        );
    }
    if dry_run {
        if !json {
            println!("dry run: nothing written");
        }
        return Ok(());
    }
    let report = imp::run(
        home,
        &store,
        &dump,
        &root,
        &imp::ImportOptions {
            with_chats,
            retag,
            root_name: None,
        },
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "located": located,
                "preview": preview,
                "report": report
            }))?
        );
        return Ok(());
    }
    println!(
        "imported {} files ({} merged with existing rows), {} file tags, {} new tags in tags.toml, {} conversations",
        report.files_imported,
        report.files_merged,
        report.file_tags,
        report.tags_added_to_file.len(),
        report.conversations
    );
    for m in report.files_missing.iter().take(20) {
        println!("  missing at import: {m}");
    }
    if report.files_missing.len() > 20 {
        println!("  and {} more", report.files_missing.len() - 20);
    }
    println!(
        "{} files queued for extraction: run `georag index` to index them (the tag stage is skipped for imported files)",
        report.jobs_queued
    );
    Ok(())
}
