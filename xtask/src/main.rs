//! Repository tasks, run with `cargo xtask <task>`.
//!
//! `bundle`   packages `georag`, `engine/` and `config/*.toml.default` for one
//!            flavour into `georag-<version>-<flavour>.tar.gz` (or `.zip` on
//!            Windows), handoff section 11.1.
//! `manifest` writes `SHA256SUMS`, `manifest.json` and `notes.md` for a
//!            directory of bundles.
#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

/// Version of the workspace, which xtask shares.
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "xtask", about = "GeoRAG repository tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Package a release bundle for one flavour
    Bundle {
        /// Flavour string, for example aarch64-apple-darwin or x86_64-pc-windows-msvc-cuda
        #[arg(long)]
        flavour: String,
        /// Built binary; default target/release/georag(.exe)
        #[arg(long)]
        bin: Option<PathBuf>,
        /// Directory whose contents become engine/ in the bundle
        #[arg(long)]
        engine: Option<PathBuf>,
        /// Output directory
        #[arg(long, default_value = "dist")]
        out: PathBuf,
        /// Version to stamp; default the workspace version
        #[arg(long, default_value = VERSION)]
        version: String,
    },
    /// Download the pdfium library for a platform into a directory
    FetchPdfium {
        /// mac-arm64 | mac-x64 | win-x64 | linux-x64 | linux-arm64 (default: this machine)
        #[arg(long)]
        platform: Option<String>,
        #[arg(long, default_value = "engine-staging")]
        out: PathBuf,
    },
    /// Write SHA256SUMS, manifest.json and notes.md for the bundles in a directory
    Manifest {
        #[arg(long, default_value = "dist")]
        dir: PathBuf,
        #[arg(long, default_value = VERSION)]
        version: String,
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Oldest version that may update straight to this one
        #[arg(long, default_value = "2.0.0")]
        min_version_for_update: String,
        /// Release notes source (markdown), copied to notes.md
        #[arg(long)]
        notes_file: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().task {
        Task::Bundle {
            flavour,
            bin,
            engine,
            out,
            version,
        } => bundle(&flavour, bin, engine, &out, &version),
        Task::FetchPdfium { platform, out } => fetch_pdfium(platform.as_deref(), &out),
        Task::Manifest {
            dir,
            version,
            channel,
            min_version_for_update,
            notes_file,
        } => manifest(
            &dir,
            &version,
            &channel,
            &min_version_for_update,
            notes_file,
        ),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits inside the repository")
        .to_path_buf()
}

fn is_windows(flavour: &str) -> bool {
    flavour.contains("windows")
}

fn bundle(
    flavour: &str,
    bin: Option<PathBuf>,
    engine: Option<PathBuf>,
    out: &Path,
    version: &str,
) -> Result<()> {
    let root = repo_root();
    let exe = if is_windows(flavour) {
        "georag.exe"
    } else {
        "georag"
    };
    let bin = bin.unwrap_or_else(|| root.join("target").join("release").join(exe));
    if !bin.is_file() {
        bail!(
            "binary not found at {}: build with `cargo build --release --features <flavour>` first",
            bin.display()
        );
    }

    let name = format!("georag-{version}-{flavour}");
    let stage = out.join("stage").join(&name);
    if stage.exists() {
        fs::remove_dir_all(&stage).with_context(|| format!("clearing {}", stage.display()))?;
    }
    fs::create_dir_all(&stage)?;

    fs::copy(&bin, stage.join(exe)).with_context(|| format!("copying {}", bin.display()))?;

    let engine_dir = stage.join("engine");
    fs::create_dir_all(&engine_dir)?;
    match engine {
        Some(src) if src.is_dir() => copy_tree(&src, &engine_dir)?,
        Some(src) => bail!("--engine {} is not a directory", src.display()),
        None => {}
    }
    fs::write(
        engine_dir.join("README.txt"),
        "Backend libraries, pdfium and OCR models live here. Keep this folder beside the georag executable.\n",
    )?;

    let config_dir = stage.join("config");
    fs::create_dir_all(&config_dir)?;
    fs::copy(
        root.join("crates/georag-core/defaults/settings.toml"),
        config_dir.join("settings.toml.default"),
    )
    .context("copying the default settings template")?;

    fs::create_dir_all(out)?;
    let archive = if is_windows(flavour) {
        let path = out.join(format!("{name}.zip"));
        write_zip(&stage, &name, &path)?;
        path
    } else {
        let path = out.join(format!("{name}.tar.gz"));
        write_tar_gz(&stage, &name, &path)?;
        path
    };
    let digest = sha256_file(&archive)?;
    let size = fs::metadata(&archive)?.len();
    println!("{}  {}  {} bytes", digest, archive.display(), size);
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn write_tar_gz(stage: &Path, name: &str, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.follow_symlinks(false);
    tar.append_dir_all(name, stage)
        .context("adding the staged folder to the archive")?;
    let gz = tar.into_inner()?;
    gz.finish()?;
    Ok(())
}

fn write_zip(stage: &Path, name: &str, path: &Path) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    fn add(
        zip: &mut zip::ZipWriter<File>,
        options: SimpleFileOptions,
        dir: &Path,
        prefix: &str,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let rel = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            if entry.file_type()?.is_dir() {
                zip.add_directory(&rel, options)?;
                add(zip, options, &entry.path(), &rel)?;
            } else {
                zip.start_file(&rel, options)?;
                let mut f = File::open(entry.path())?;
                io::copy(&mut f, zip)?;
            }
        }
        Ok(())
    }
    add(&mut zip, options, stage, name)?;
    zip.finish()?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn manifest(
    dir: &Path,
    version: &str,
    channel: &str,
    min_version_for_update: &str,
    notes_file: Option<PathBuf>,
) -> Result<()> {
    let prefix = format!("georag-{version}-");
    let mut assets = serde_json::Map::new();
    let mut sums = String::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with(&prefix) && (n.ends_with(".tar.gz") || n.ends_with(".zip"))
        })
        .collect();
    entries.sort();
    if entries.is_empty() {
        bail!("no bundles named {prefix}* found in {}", dir.display());
    }
    for path in &entries {
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        let flavour = file
            .trim_start_matches(&prefix)
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".zip")
            .to_string();
        let sha256 = sha256_file(path)?;
        let size = fs::metadata(path)?.len();
        sums.push_str(&format!("{sha256}  {file}\n"));
        assets.insert(
            flavour,
            serde_json::json!({ "file": file, "sha256": sha256, "size": size }),
        );
    }

    let notes = match notes_file {
        Some(p) => fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?,
        None => format!("# GeoRAG {version}\n"),
    };
    fs::write(dir.join("notes.md"), notes)?;
    fs::write(dir.join("SHA256SUMS"), sums)?;

    let published = unix_time_rfc3339();
    let manifest = serde_json::json!({
        "version": version,
        "channel": channel,
        "published": published,
        "min_version_for_update": min_version_for_update,
        "notes": "notes.md",
        "assets": assets,
    });
    let mut f = File::create(dir.join("manifest.json"))?;
    f.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    f.write_all(b"\n")?;
    println!(
        "wrote {} assets to {}",
        entries.len(),
        dir.join("manifest.json").display()
    );
    Ok(())
}

/// Current UTC time as RFC3339 without pulling in a date crate.
fn unix_time_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// pdfium build that pdfium-render 0.9.4 binds against (feature pdfium_7881).
const PDFIUM_BUILD: &str = "7881";

fn fetch_pdfium(platform: Option<&str>, out: &Path) -> Result<()> {
    let platform = match platform {
        Some(p) => p.to_string(),
        None => match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "mac-arm64".into(),
            ("macos", _) => "mac-x64".into(),
            ("windows", _) => "win-x64".into(),
            ("linux", "aarch64") => "linux-arm64".into(),
            _ => "linux-x64".into(),
        },
    };
    let url = format!(
        "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F{PDFIUM_BUILD}/pdfium-{platform}.tgz"
    );
    fs::create_dir_all(out)?;
    let archive = out.join("pdfium.tgz");
    // curl is present on every CI runner and on macOS, Windows 10+ and Linux.
    let status = std::process::Command::new("curl")
        .args(["-sSL", "--fail", "-o"])
        .arg(&archive)
        .arg(&url)
        .status()
        .context("running curl")?;
    if !status.success() {
        bail!("download of {url} failed");
    }
    let file = File::open(&archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    let mut found = None;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if (path.starts_with("lib") || path.starts_with("bin"))
            && matches!(
                name.as_str(),
                "libpdfium.dylib" | "libpdfium.so" | "pdfium.dll"
            )
        {
            let target = out.join(&name);
            entry.unpack(&target)?;
            found = Some(target);
        }
    }
    fs::remove_file(&archive).ok();
    match found {
        Some(p) => {
            println!("pdfium {PDFIUM_BUILD} for {platform}: {}", p.display());
            Ok(())
        }
        None => bail!("the archive held no pdfium library"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavour_decides_archive_type() {
        assert!(is_windows("x86_64-pc-windows-msvc-cuda"));
        assert!(!is_windows("aarch64-apple-darwin"));
    }

    #[test]
    fn rfc3339_has_the_expected_shape() {
        let t = unix_time_rfc3339();
        assert_eq!(t.len(), 20);
        assert!(t.ends_with('Z'));
        assert!(t.starts_with("20"));
    }
}
