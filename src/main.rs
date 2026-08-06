//! oedb-build: generates a static OpenEventDatabase-compatible instance for
//! Vaucluse (FR-84) from Bison Futé (DATEX II), curated YAML files and
//! approved GitHub issues.
//!
//! Usage: `oedb-build [--out DIR]`
//!
//! Environment:
//! - `OEDB_OFFLINE=1`     skip all network calls (curated files only)
//! - `OEDB_DATEX_URL`     override the Bison Futé feed URL
//! - `GITHUB_TOKEN`       token used to read event issues (optional)
//! - `GITHUB_REPOSITORY`  issues repository (default `thepriben/oedb-rs`)

mod emit;
mod ingest;
mod model;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

const USER_AGENT: &str = "oedb-rs/0.1 (+https://github.com/thepriben/oedb-rs)";

fn parse_out_dir() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for pair in args.windows(2) {
        if pair[0] == "--out" {
            return PathBuf::from(&pair[1]);
        }
    }
    PathBuf::from("dist")
}

fn copy_site(out: &Path) -> Result<()> {
    let site = Path::new("site");
    if !site.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(site).context("reading site/")? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), out.join(entry.file_name()))
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let out = parse_out_dir();
    let now = Utc::now();
    let offline = std::env::var("OEDB_OFFLINE").map(|v| v == "1").unwrap_or(false);
    let mut events = Vec::new();

    // 1. Curated events (manual curation + persisted issue events).
    let curated = ingest::curated::load_dir(Path::new("events/curated"), now)?;
    println!("curated: {} événement(s)", curated.len());
    events.extend(curated);

    let persisted = ingest::curated::load_dir(Path::new("events/from-issues"), now)?;
    println!("from-issues (persistés): {} événement(s)", persisted.len());
    events.extend(persisted);

    if !offline {
        // 2. Approved GitHub issues, persisted for future builds.
        let repo = std::env::var("GITHUB_REPOSITORY")
            .unwrap_or_else(|_| "thepriben/oedb-rs".to_string());
        let token = std::env::var("GITHUB_TOKEN").ok();
        match ingest::issues::fetch(&repo, token.as_deref(), USER_AGENT) {
            Ok(issue_events) => {
                println!("issues approuvées: {} événement(s)", issue_events.len());
                if ingest::issues::persist(Path::new("events/from-issues"), &issue_events)? {
                    println!("events/from-issues: fichiers mis à jour");
                }
                for issue_event in issue_events {
                    match issue_event.into_event(now) {
                        Ok(event) => events.push(event),
                        Err(error) => eprintln!("issue ignorée: {error:#}"),
                    }
                }
            }
            Err(error) => eprintln!("lecture des issues impossible: {error:#}"),
        }

        // 3. Bison Futé DATEX II, filtered on Vaucluse.
        let datex_url = std::env::var("OEDB_DATEX_URL")
            .unwrap_or_else(|_| ingest::datex::DEFAULT_FEED_URL.to_string());
        match ingest::datex::fetch(&datex_url, USER_AGENT) {
            Ok(datex_events) => {
                println!("bison futé (FR-84): {} événement(s)", datex_events.len());
                events.extend(datex_events);
            }
            Err(error) => eprintln!("flux Bison Futé indisponible: {error:#}"),
        }
    } else {
        println!("OEDB_OFFLINE=1: réseau désactivé");
    }

    let consolidated = emit::consolidate(events, now);
    let summary = emit::emit(&out, &consolidated, now)?;
    copy_site(&out)?;

    println!(
        "{}: {} événement(s) publiés — {}",
        out.display(),
        summary.total,
        summary
            .by_what
            .iter()
            .map(|(what, count)| format!("{what}: {count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
