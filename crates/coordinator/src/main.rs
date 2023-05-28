mod cli;

use clap::Parser;
use cli::Options;

fn main() -> eyre::Result<()> {
    cli::run(Options::parse())
}
