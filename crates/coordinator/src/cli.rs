use colored::Colorize;
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
    #[clap(long, action, default_value = "1000")]
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

    // 2) Loop and run for repeat times
    while options.inf_run || iteration < options.repeat.into() {
        let gen_opts = data_race_generator::GenOptions {
            seed: OsRng.gen(),
            workgroup_size: options.workgroup_size,
            racy_loc_pct: options.racy_loc_pct,
            racy_constant_loc_pct: options.racy_constant_loc_pct,
            racy_var_pct: options.racy_var_pct,
            num_lits: options.num_lits,
            stmts: options.stmts,
            vars: options.vars,
            locs_per_thread: options.locs_per_thread,
            constant_locs: options.constant_locs,
        };

        let shaders = data_race_generator::gen(gen_opts);
        let input_size = ((options.workgroup_size * options.workgroups * options.locs_per_thread)
            + options.constant_locs)
            * 4; // Mult by 4 since u8
        let mut rng = rand::thread_rng();
        let random_data: Vec<u8> = (0..input_size).map(|_| rng.gen_range(0..u8::MAX)).collect();
        let mut input_data = HashMap::new();
        input_data.insert("0:0".to_owned(), random_data);

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
            workgroups: options.workgroups,
            workgroup_size: options.workgroup_size,
            reps: options.config_rep,
        };

        let mismatches = data_race_runner::execute(
            racy_shader,
            safe_shader,
            &shaders.info,
            &input_data,
            exec_options,
        );

        if mismatches.len() == 0 {
            println!("Iteration {}: {}", iteration, "Configs match".green());
        } else {
            println!("Iteration {}: {}", iteration, "Configs mismatch".red());

            let folder_path = options.target.clone() + &"/".to_owned() + &iteration.to_string();
            fs::create_dir_all(&folder_path)?;

            let safe_path = folder_path.clone()
                + &"/safe_".to_owned()
                + &iteration.to_string()
                + &".wgsl".to_owned();
            let safe_output = Box::new(BufWriter::new(File::create(safe_path)?));
            ast::writer::Writer::default().write_module_default(safe_output, &shaders.safe)?;

            let race_path = folder_path.clone()
                + &"/race_".to_owned()
                + &iteration.to_string()
                + &".wgsl".to_owned();
            let race_output = Box::new(BufWriter::new(File::create(race_path)?));
            ast::writer::Writer::default().write_module_default(race_output, &shaders.race)?;

            let info_path = folder_path.clone() + &"/info.json".to_owned();
            let mut info_output = Box::new(BufWriter::new(File::create(info_path)?));
            writeln!(info_output, "{}", to_string(&shaders.info)?)?;

            let input_path = folder_path.clone() + &"/input.json".to_owned();
            let mut input_output = Box::new(BufWriter::new(File::create(input_path)?));
            writeln!(input_output, "{}", to_string(&input_data)?)?;

            let mismatches_path = folder_path.clone() + &"/errors.out".to_owned();
            let mut mismatches_output = Box::new(BufWriter::new(File::create(mismatches_path)?));
            for mismatch in mismatches {
                println!("{:?}", mismatch);
                writeln!(mismatches_output, "{}", to_string(&mismatch)?)?;
            }
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
