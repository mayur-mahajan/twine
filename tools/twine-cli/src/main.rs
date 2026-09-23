//! `twine`: Twine's font and image converters.

use clap::{Parser, Subcommand};

/// Twine asset converters.
#[derive(Debug, Parser)]
#[command(name = "twine", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Generate a Rust bitmap font from a TTF file.
    Font(twine_cli::font::FontArgs),
    /// Convert an image (PNG, JPEG, BMP, GIF, QOI) into a Rust `Image` in any color format.
    Image(twine_cli::image::ImageArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match &cli.cmd {
        Cmd::Font(args) => twine_cli::font::run(args),
        Cmd::Image(args) => twine_cli::image::run(args),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
