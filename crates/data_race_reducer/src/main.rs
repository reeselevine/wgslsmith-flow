use clap::Parser;
use data_race_reducer::cli::{self, RunOptions};

fn main() -> eyre::Result<()> {
    cli::run(RunOptions::parse())
}
