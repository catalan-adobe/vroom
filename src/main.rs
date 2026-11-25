mod model;
mod paths;
mod tui;
mod video;

use std::{env, fs, path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Keep the original CLI diagnostics, including the legacy name.
            eprintln!("vedit: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let path = PathBuf::from(env::args_os().nth(1).context("usage: vedit <video-file>")?);
    fs::metadata(&path).with_context(|| format!("open {path:?}"))?;

    eprintln!("Probing {}…", path.display());
    let info = video::probe(&path).context("probe")?;
    let sidecar = paths::save_path(&path);
    let mut project = model::Project::new(path, info);
    if sidecar.exists() {
        match project.load(&sidecar) {
            Ok(()) => eprintln!("Resumed from {}", sidecar.display()),
            Err(error) => eprintln!("warning: could not load {}: {error:#}", sidecar.display()),
        }
    }

    tui::run(project).await
}
