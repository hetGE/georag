//! `georag`: the single binary. No arguments opens the TUI; subcommands run
//! the same core in-process (and through the daemon once P3 lands).

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
        None => {
            let _logs = logging::init(&home, false).ok();
            georag_tui::run(&home)?;
            Ok(())
        }
    }
}
