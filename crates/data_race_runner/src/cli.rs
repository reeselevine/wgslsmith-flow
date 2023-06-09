use clap::Parser;
use data_race_generator::{DataRaceInfo, RaceValueStrategy};
use types::ConfigId;
use colored::Colorize;

use std::fs::File;
use std::io::Read;

use eyre::{eyre, Context};

use std::collections::HashMap;

use crate::ExecOptions;



#[derive(Parser)]
pub struct RunOptions {
    /// Path to racy shader program to be executed
    #[clap(action)]
    pub racy_shader: String,

    /// Path to safe shader program to be executed
    #[clap(action)]
    pub safe_shader: String,

    #[clap(action)]
    /// Path to information about safe/racy memory locations
    pub data_race_info: String,

    /// Input data for buffers. If None, random data will be generated.
    #[clap(action)]
    pub input_data: Option<String>,

    /// List of configurations to test.
    ///
    /// Configurations must be specified using their IDs. Use the `harness list` command to see available
    /// configurations.
    ///
    /// If no configurations are provided, defaults will be selected for the execution platform.
    #[clap(short, long = "config", action)]
    pub configs: Vec<ConfigId>,

    /// Number of workgroups
    #[clap(long, action, default_value = "1")]
    pub workgroups: u32,

    /// Workgroup size 
    #[clap(long, action, default_value = "1")]
    pub workgroup_size: u32,

    /// Number of times to run the shaders 
    #[clap(long, action, default_value = "1")]
    pub reps: u32

}

pub fn run(options: RunOptions) -> eyre::Result<()> {
    let configs = if options.configs.len() == 0 {
        // Default to all configurations
        harness::query_configs().into_iter().map(|config| config.id).collect()
    } else {
        options.configs.clone()
    };
    print_configs(&configs);

    let safe_shader = read_shader_from_path(&options.safe_shader)?;
    let racy_shader = read_shader_from_path(&options.racy_shader)?;
    let data_race_info: DataRaceInfo = serde_json::from_reader(File::open(&options.data_race_info)?)?;
    let input_size = ((options.workgroup_size * options.workgroups * data_race_info.locs_per_thread) + data_race_info.constant_locs) * 4; // Mult by 4 since u8
    let input_data = get_input_data(options.input_data, &data_race_info, input_size)?; 

    let exec_options = ExecOptions {
      configs,
      workgroups: options.workgroups,
      workgroup_size: options.workgroup_size,
      reps: options.reps
    };

    let result = crate::execute(racy_shader, safe_shader, &data_race_info, &input_data, exec_options);
    if result.len() == 0 {
        println!("{}", "Configs match".green());
    } else {
        println!("{}", "Configs don't match".red());
    }
    for mismatch in result {
      println!("{:?}", mismatch);
    }
    
    Ok(())
}

fn get_input_data(input_data: Option<String>, data_race_info: &DataRaceInfo, size: u32) -> eyre::Result<HashMap<String, Vec<u8>>> {
  match input_data {
    Some(input_data) => match File::open(input_data) {
      Ok(file) => serde_json::from_reader(file)
                        .wrap_err_with(|| eyre!("failed to parse input data")),
      Err(e) => Err(e.into()),
    },
    None => {
      // if no input passed, initialize all data to 1 (2 if using even strategy)
        let random_data: Vec<u8> = match data_race_info.race_val_strat {
          Some(RaceValueStrategy::Even) => (0..size).map(|i| if i % 4  == 0 { 2 } else { 0 } ).collect(),
          None => (0..size).map(|i| if i % 4  == 0 { 1 } else { 0 } ).collect()
        };
      let mut map = HashMap::new();
      map.insert("0:0".to_owned(), random_data);
      Ok(map)
    }
  }
}

fn read_shader_from_path(path: &str) -> eyre::Result<String> {
    let mut input: Box<dyn Read> = match path {
        "-" => Box::new(std::io::stdin()),
        path => Box::new(File::open(path)?),
    };

    let mut shader = String::new();
    input.read_to_string(&mut shader)?;

    Ok(shader)
}

fn print_configs(configs: &[ConfigId]) {
  println!("Using configs:");
  for (_, config) in configs.iter().enumerate() {
    println!("  {config}");
  }
}
