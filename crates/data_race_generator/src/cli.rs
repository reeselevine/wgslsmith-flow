use std::fs::File;
use std::hash::BuildHasher;
use std::io::{self, BufWriter};
use std::path::Path;
use std::rc::Rc;

use ast::Module;
use clap::Parser;
use hashers::fx_hash::FxHasher;

use serde_json::{to_string};

use rand::prelude::StdRng;
use rand::rngs::OsRng;
use rand::{Rng, SeedableRng};

use crate::{GenOptions, RaceValueStrategy};

#[derive(Parser)]
pub struct Options {
    /// Optional u64 to seed the random generator
    #[clap(action)]
    pub seed: Option<u64>,

    /// Workgroup size 
    #[clap(long, action, default_value = "1")]
    pub workgroup_size: u32,

    /// Race value strategy
    #[clap(long, action)]
    pub race_value_strategy: Option<RaceValueStrategy>,

    /// Percentage of memory locations which can participate in races 
    #[clap(long, action, default_value = "30")]
    pub racy_loc_pct: u32,

    /// Percentage of constant memory locations which can participate in races 
    #[clap(long, action, default_value = "30")]
    pub racy_constant_loc_pct: u32,

    /// Percentage of local variables which can participate in races 
    #[clap(long, action, default_value = "30")]
    pub racy_var_pct: u32,

    /// Maximum number of statements in a conditional block
    #[clap(long, action, default_value = "100")]
    pub block_max_stmts: u32,

    /// Maximum number of nesting of conditionals
    #[clap(long, action, default_value = "3")]
    pub block_max_nest_level: u32,

    /// Percent chance for an if statement to have an else
    #[clap(long, action, default_value = "30")]
    pub else_chance: u32,

    /// Maximum number of loop iterations
    #[clap(long, action, default_value = "10")]
    pub max_loop_iter: u32,

    /// Number of literals to generate
    #[clap(long, action, default_value = "4")]
    pub num_lits: u32,

    /// Number of statements to generate
    #[clap(long, action, default_value = "8")]
    pub stmts: u32,

    /// Number of local variables to generate
    #[clap(long, action, default_value = "8")]
    pub vars: u32,

    /// Number of uninitialized local variables to generate
    #[clap(long, action, default_value = "8")]
    pub uninit_vars: u32,

    /// Number of memory locations associated with each thread 
    #[clap(long, action, default_value = "8")]
    pub locs_per_thread: u32,

    /// Number of constant memory locations
    #[clap(long, action, default_value = "16")]
    pub constant_locs: u32,

    /// Path to output file (use `-` for stdout)
    #[clap(short, long, action, default_value = "-")]
    pub output: String,

    /// Percentage of time racy statements can go out of bounds
    #[clap(long, action, default_value = "0")]
    pub oob_pct: u32,

    /// The size of the buffer to try to read out of bounds 
    #[clap(long, action, default_value = "256")]
    pub data_buf_size: u32,

    /// The number of times a oob pattern is attempted
    #[clap(long, action, default_value = "3")]
    pub pattern_slots: u32,

    /// Try and maximize register pressure
    #[clap(long, action, default_value = "false")]
    pub reg_pressure: bool,
}

#[derive(Clone, Debug)]
struct BuildFxHasher;

impl BuildHasher for BuildFxHasher {
    type Hasher = FxHasher;

    fn build_hasher(&self) -> Self::Hasher {
        FxHasher::default()
    }
}

fn output_helper(options: Rc<Options>, source: Module, prefix: &str, seed: u64) -> eyre::Result<()> {
    // Create a writer to file input and prepend "race_<filename>.wgsl"
    let mut output: Box<dyn io::Write> = if options.output == "-" {
        Box::new(io::stdout())
    } else {
        if let Some(dir) = Path::new(&options.output).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let out = options.output.rsplit_once('/');
        let new_path = match out {
                         Some((a, b)) => a.to_owned() + "/" + prefix + "_" + b,
                         None => prefix.to_owned() + "_" + &options.output
                       };
        Box::new(BufWriter::new(File::create(new_path)?))
    };

    writeln!(output, "// Seed: {seed}")?;
    writeln!(output)?;
    ast::writer::Writer::default().write_module_default(output, &source)?;
    Ok(())
   
}

pub fn run(options: Options) -> eyre::Result<()> {
    let options = Rc::new(options);

    let seed = match options.seed {
        Some(seed) => seed,
        None => OsRng.gen(),
    };

    tracing::info!("generating shader from seed: {}", seed);

    let mut rng = StdRng::seed_from_u64(seed);
    let gen_options = GenOptions {
      seed,
      workgroup_size: options.workgroup_size,
      racy_loc_pct: options.racy_loc_pct,
      racy_constant_loc_pct: options.racy_constant_loc_pct,
      racy_var_pct: options.racy_var_pct,
      block_max_stmts: options.block_max_stmts,
      block_max_nest_level: options.block_max_nest_level,
      else_chance: options.else_chance,
      max_loop_iter: options.max_loop_iter,
      num_lits: options.num_lits,
      stmts: options.stmts,
      vars: options.vars,
      uninit_vars: options.uninit_vars,
      locs_per_thread: options.locs_per_thread,
      constant_locs: options.constant_locs,
      race_val_strat: options.race_value_strategy,
      pattern_weights: (16, 14, 14, 14, 14, 14, 14),
      oob_pct: options.oob_pct,
      data_buf_size: options.data_buf_size,
      pattern_slots: options.pattern_slots,
      reg_pressure: options.reg_pressure
    };
    let out = crate::Generator::new(&mut rng, &gen_options).gen_module();
    
    let safe_shader = out.safe;
    let race_shader = out.race;
    let data_race_info = out.info;

    output_helper(options.clone(), race_shader, "race", seed)?;
    output_helper(options.clone(), safe_shader, "safe", seed)?;

    let mut output: Box<dyn io::Write> = if options.output == "-" {
        Box::new(io::stdout())
    } else {
        if let Some(dir) = Path::new(&options.output).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let out = options.output.rsplit_once('/');
        let new_path = match out {
                         Some((a, _)) => a.to_owned() + "/info.json",
                         None => "info.json".to_string()
                       };
        Box::new(BufWriter::new(File::create(new_path)?))
    };

    writeln!(output, "{}", to_string(&data_race_info)?)?;

    Ok(())
}
