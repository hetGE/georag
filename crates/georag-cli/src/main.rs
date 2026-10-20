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
