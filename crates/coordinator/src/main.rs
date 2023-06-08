use clap::Parser;

use coordinator::cli::{self, Options};

fn main() -> eyre::Result<()> {
    cli::run(Options::parse())
}
