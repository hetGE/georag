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
