use clap::StructOpt;
use data_race_generator::cli::{self, Options};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    cli::run(Options::parse())
}
