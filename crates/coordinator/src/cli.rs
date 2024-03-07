use colored::Colorize;
use data_race_generator::GenOptions;
use data_race_generator::RaceValueStrategy;
use data_race_runner::MismatchType;
use rand::thread_rng;
use reflection_types::BufferInitInfo;
use serde_json::to_string;
use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;

use clap::Parser;

use types::ConfigId;

use rand::rngs::OsRng;
use rand::Rng;

use std::collections::HashMap;
use std::io;

#[derive(Parser)]
pub struct Options {
    /// Location that files will be put in, by default working directory
    #[clap(action, default_value = ".")]
    pub target: String,

    /// Number of times for files to be generated and tested
    #[clap(long, action, default_value = "5")]
    pub repeat: u32,

    /// Number of times for a file to be run per config
    #[clap(long, action, default_value = "3")]
    pub config_rep: u32,

    /// Optional bool to cause indefinite looping (overrides the repeat option)
    #[clap(short, long, action)]
    pub inf_run: bool,

    /// Optional bool to generate random options (overrides all other options)
    #[clap(short, long, action)]
    pub random_opts: bool,

    /// List of configurations to test.
    ///
    /// Configurations must be specified using their IDs. Use the harness 'list' command to see available
    /// configurations.
    ///
    /// If no configurations are provided, defaults will be selected for the execution platform
    #[clap(short, long = "config", action)]
    pub configs: Vec<ConfigId>,

    /// Number of workgroups
    #[clap(long, action, default_value = "1")]
    pub workgroups: u32,

    /// Workgroup size
    #[clap(long, action, default_value = "1")]
    pub workgroup_size: u32,

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

    /// Percent chance to generate an else in an if statement
    #[clap(long, action, default_value = "30")]
    pub else_chance: u32,

    /// Maximum number of loop iterations
    #[clap(long, action, default_value = "10")]
    pub max_loop_iter: u32,

    /// Number of literals to generate
    #[clap(long, action, default_value = "4")]
    pub num_lits: u32,

    /// Number of statements to generate
    #[clap(long, action, default_value = "1000")]
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

    /// Race value strategy
    #[clap(long, action)]
    pub race_value_strategy: Option<RaceValueStrategy>,

    /// Percentage of time racy statements can go out of bounds
    #[clap(long, action, default_value = "0")]
    pub oob_pct: u32,

    /// When generating random shaders, whether to disable out of bounds accesses
    #[clap(long, action)]
    pub disable_oob: bool,

    /// Size of buffer to read out of bounds from
    #[clap(long, action, default_value = "256")]
    pub data_buf_size: u32,

    /// Number of pattern slots
    #[clap(long, action, default_value = "3")]
    pub pattern_slots: u32,

    /// The mismatch types to check
    #[clap(long, action)]
    pub check_mismatches: Vec<MismatchType>,
}

fn random_opts(disable_oob: bool) -> GenOptions {
    let mut rng = thread_rng();
    GenOptions {
        seed: rng.gen_range(0..=18446744073709551615),
        workgroup_size: rng.gen_range(1..=128),
        racy_loc_pct: rng.gen_range(0..=100),
        racy_constant_loc_pct: rng.gen_range(0..=100),
        racy_var_pct: rng.gen_range(0..=100),
        block_max_stmts: rng.gen_range(2..=100),
        block_max_nest_level: 3,
        else_chance: rng.gen_range(0..=100),
        max_loop_iter: 10,
        num_lits: rng.gen_range(1..=16),
        stmts: rng.gen_range(0..=1000),
        vars: rng.gen_range(1..=16),
        uninit_vars: rng.gen_range(1..=16),
        locs_per_thread: rng.gen_range(1..=16),
        constant_locs: rng.gen_range(1..=16),
        oob_pct: if disable_oob {
            0
        } else {
            rng.gen_range(0..=100)
        },
        race_val_strat: if rng.gen_range(0..=100) > 50 {
            None
        } else {
            Some(RaceValueStrategy::Even)
        },
        data_buf_size: rng.gen_range(32..=1024),
        pattern_slots: rng.gen_range(1..=5),
    }
}

pub fn run(options: Options) -> eyre::Result<()> {
    // 1) Construct a data race gen Options struct
    let mut iteration: u128 = 0;

    let configs = if options.configs.len() == 0 {
        // Default to all configurations
        harness::query_configs()
            .into_iter()
            .map(|config| config.id)
            .collect()
    } else {
        options.configs.clone()
    };
    print_configs(&configs);

    // Default to all mismatch types
    let mismatches_to_check = if options.check_mismatches.len() == 0 {
        vec![
            MismatchType::ConstantLocation,
            MismatchType::SafeLocation,
            MismatchType::UninitializedVar,
            MismatchType::OobRead,
        ]
    } else {
        options.check_mismatches.clone()
    };

    // 2) Loop and run for repeat times
    while options.inf_run || iteration < options.repeat.into() {
        let gen_opts = if options.random_opts {
            random_opts(options.disable_oob)
        } else {
            GenOptions {
                seed: OsRng.gen(),
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
                oob_pct: options.oob_pct,
                data_buf_size: options.data_buf_size,
                pattern_slots: options.pattern_slots,
            }
        };

        let workgroups = if options.random_opts {
            thread_rng().gen_range(1..=128)
        } else {
            options.workgroups
        };

        match data_race_generator::gen_result(&gen_opts) {
            Ok(shaders) => {
                let input_size =
                    ((gen_opts.workgroup_size * workgroups * gen_opts.locs_per_thread)
                        + gen_opts.constant_locs)
                        * 4; // Mult by 4 since u8
                let random_data: Vec<u8> = match shaders.info.race_val_strat {
                    Some(RaceValueStrategy::Even) => (0..input_size)
                        .map(|i| if i % 4 == 0 { 2 } else { 0 })
                        .collect(),
                    None => (0..input_size)
                        .map(|i| if i % 4 == 0 { 1 } else { 0 })
                        .collect(),
                };
                let pattern_data_buffer: Vec<u8> = (0..(gen_opts.data_buf_size * 4))
                    .map(|i| if i % 4 == 0 { 1 } else { 0 })
                    .collect();
                let mut input_info = HashMap::new();
                input_info.insert("0:0".to_owned(), BufferInitInfo::Data { data: random_data }); // mem buffer
                input_info.insert(
                    "0:1".to_owned(),
                    BufferInitInfo::Size {
                        size: gen_opts.workgroup_size * workgroups * gen_opts.uninit_vars * 4,
                    },
                ); // uninit vars buffer
                input_info.insert(
                    "0:2".to_owned(),
                    BufferInitInfo::Size {
                        size: gen_opts.workgroup_size * workgroups * gen_opts.pattern_slots * 4,
                    },
                ); // index pattern buffer
                input_info.insert(
                    "0:3".to_owned(),
                    BufferInitInfo::Data {
                        data: pattern_data_buffer,
                    },
                ); // data pattern buffer
                input_info.insert(
                    "0:4".to_owned(),
                    BufferInitInfo::Size {
                        size: gen_opts.workgroup_size * workgroups * gen_opts.pattern_slots * 4,
                    },
                ); // output pattern buffer

                let mut racy_buf = Vec::new();
                let racy_output: Box<dyn io::Write> = Box::new(&mut racy_buf);
                ast::writer::Writer::default().write_module_default(racy_output, &shaders.race)?;
                let racy_shader = String::from_utf8(racy_buf)?;

                let mut safe_buf = Vec::new();
                let safe_output: Box<dyn io::Write> = Box::new(&mut safe_buf);
                ast::writer::Writer::default().write_module_default(safe_output, &shaders.safe)?;
                let safe_shader = String::from_utf8(safe_buf)?;

                let exec_options = data_race_runner::ExecOptions {
                    configs: configs.clone(),
                    workgroups: workgroups,
                    workgroup_size: gen_opts.workgroup_size,
                    reps: options.config_rep,
                    check_mismatches: mismatches_to_check.clone(),
                };

                let mismatches = data_race_runner::execute(
                    racy_shader,
                    safe_shader,
                    &shaders.info,
                    &input_info,
                    exec_options,
                );

                if mismatches.len() == 0 {
                    println!("Iteration {}: {}", iteration, "Configs match".green());
                } else {
                    println!("Iteration {}: {}", iteration, "Configs mismatch".red());

                    let folder_path =
                        options.target.clone() + &"/".to_owned() + &iteration.to_string();
                    fs::create_dir_all(&folder_path)?;

                    let safe_path = folder_path.clone()
                        + &"/safe_".to_owned()
                        + &iteration.to_string()
                        + &".wgsl".to_owned();
                    let safe_output = Box::new(BufWriter::new(File::create(safe_path)?));
                    ast::writer::Writer::default()
                        .write_module_default(safe_output, &shaders.safe)?;

                    let race_path = folder_path.clone()
                        + &"/race_".to_owned()
                        + &iteration.to_string()
                        + &".wgsl".to_owned();
                    let race_output = Box::new(BufWriter::new(File::create(race_path)?));
                    ast::writer::Writer::default()
                        .write_module_default(race_output, &shaders.race)?;

                    let info_path = folder_path.clone() + &"/info.json".to_owned();
                    let mut info_output = Box::new(BufWriter::new(File::create(info_path)?));
                    writeln!(info_output, "{}", to_string(&shaders.info)?)?;

                    let input_path = folder_path.clone() + &"/input.json".to_owned();
                    let mut input_output = Box::new(BufWriter::new(File::create(input_path)?));
                    writeln!(input_output, "{}", to_string(&input_info)?)?;

                    let mismatches_path = folder_path.clone() + &"/errors.out".to_owned();
                    let mut mismatches_output =
                        Box::new(BufWriter::new(File::create(mismatches_path)?));
                    for mismatch in mismatches {
                        println!("{:?}", mismatch);
                        writeln!(mismatches_output, "{}", to_string(&mismatch)?)?;
                    }
                }
            }
            Err(failure) => println!("Generation failure: {}", failure.reason),
        }
        iteration += 1;
    }
    Ok(())
}

fn print_configs(configs: &[ConfigId]) {
    println!("Using configs:");
    for (_, config) in configs.iter().enumerate() {
        println!("  {config}");
    }
}
