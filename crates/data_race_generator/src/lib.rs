mod data_race_gen;

use std::collections::HashMap;
use std::fs::File;
use std::hash::BuildHasher;
use std::io::{self, BufWriter};
use std::path::Path;
use std::rc::Rc;

use ast::{StorageClass, VarQualifier, Module};
use clap::Parser;
use eyre::{eyre};
use hashers::fx_hash::FxHasher;

pub use data_race_gen::{Generator, Outputs};
use rand::prelude::StdRng;
use rand::rngs::OsRng;
use rand::{Rng, SeedableRng};

#[derive(Parser)]
pub struct Options {
    /// Optional u64 to seed the random generator
    #[clap(action)]
    pub seed: Option<u64>,

    /// Print ast instead of WGSL code
    #[clap(short, long, action)]
    pub debug: bool,

    /// Logging configuration string (see https://docs.rs/tracing-subscriber/0.3.7/tracing_subscriber/struct.EnvFilter.html#directives)
    #[clap(long, action)]
    pub log: Option<String>,

    /// Workgroup size 
    #[clap(long, action, default_value = "1")]
    pub workgroup_size: u32,

    /// Percentage of memory locations which can participate in races 
    #[clap(long, action, default_value = "50")]
    pub racy_loc_pct: u32,

    /// Percentage of constant memory locations which can participate in races 
    #[clap(long, action, default_value = "50")]
    pub racy_constant_loc_pct: u32,

    /// Percentage of local variables which can participate in races 
    #[clap(long, action, default_value = "50")]
    pub racy_var_pct: u32,

    /// Number of literals to generate
    #[clap(long, action, default_value = "4")]
    pub num_lits: u32,

    /// Number of statements to generate
    #[clap(long, action, default_value = "8")]
    pub stmts: u32,

    /// Number of local variables to generate
    #[clap(long, action, default_value = "8")]
    pub vars: u32,

    /// Number of memory locations associated with each thread 
    #[clap(long, action, default_value = "8")]
    pub locs_per_thread: u32,

    /// Number of constant memory locations
    #[clap(long, action, default_value = "16")]
    pub constant_locs: u32,

    /// Path to output file (use `-` for stdout)
    #[clap(short, long, action, default_value = "-")]
    pub output: String,
}

#[derive(Clone, Debug)]
struct BuildFxHasher;

impl BuildHasher for BuildFxHasher {
    type Hasher = FxHasher;

    fn build_hasher(&self) -> Self::Hasher {
        FxHasher::default()
    }
}

fn output_helper(options: Rc<Options>, source: Module, prefix: &str, rng: &mut StdRng, seed: u64) -> eyre::Result<()> {
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

    if !options.debug {
        let mut init_data = HashMap::new();

        for var in &source.vars {
            if let Some(VarQualifier { storage_class, .. }) = &var.qualifier {
                if *storage_class != StorageClass::Uniform {
                    continue;
                }

                let type_desc = common::Type::try_from(&var.data_type).map_err(|e| eyre!(e))?;

                let group = var.group_index().unwrap();
                let binding = var.binding_index().unwrap();

                let size = type_desc.buffer_size();
                let data: Vec<u8> = (0..size).map(|_| rng.gen()).collect();

                init_data.insert(format!("{group}:{binding}"), data);
            }
        }

        let init_data = serde_json::to_string(&init_data)?;

        writeln!(output, "// {init_data}")?;
        writeln!(output, "// Seed: {seed}")?;
        writeln!(output)?;
    }

    if options.debug {
        writeln!(output, "{source:#?}")?;
    } else {
        struct Output<'a>(&'a mut dyn std::io::Write);

        impl<'a> std::fmt::Write for Output<'a> {
            fn write_str(&mut self, s: &str) -> std::fmt::Result {
                self.0.write_all(s.as_bytes()).unwrap();
                Ok(())
            }
        }

        ast::writer::Writer::default().write_module(&mut Output(&mut output), &source)?;
    }
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
    let out = Generator::new(&mut rng, options.clone()).gen_module();
    
    let safe_shader = out.safe;
    let race_shader = out.race;
    let json_info = out.info;

    output_helper(options.clone(), race_shader, "race", &mut rng, seed)?;
    output_helper(options.clone(), safe_shader, "safe", &mut rng, seed)?;

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

    writeln!(output, "{json_info}")?;

    Ok(())
}
