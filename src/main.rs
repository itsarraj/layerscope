use std::fs;
use std::path::PathBuf;

use clap::Parser;
use layerscope::{report, tarutil};

#[derive(Parser)]
#[command(
    name = "layerscope",
    about = "A container image layer/bloat analyzer — a dive alternative"
)]
struct Cli {
    /// A tarball from `docker save myimage -o image.tar` (or `docker save myimage | cat > image.tar`).
    image_tar: PathBuf,

    /// How many of the largest files to list per layer.
    #[arg(long, default_value_t = 10)]
    top: usize,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let bytes = fs::read(&cli.image_tar)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", cli.image_tar.display()))?;

    let entries = tarutil::read_tar_entries(&bytes)?;
    let images = report::analyze_image_tar(&entries, cli.top)?;

    for image in &images {
        print!("{}", report::render_text(image));
        println!();
    }
    Ok(())
}
