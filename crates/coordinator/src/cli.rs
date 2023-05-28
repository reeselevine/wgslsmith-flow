mod utils;

use std::fs::File;
use std::fs;
use std::io::Read;
use std::io::BufWriter;
use std::path::Path;
use serde_json::json;
use serde::Deserialize;
use colored::Colorize;
use std::io::Write;

use clap::Parser;

use types::ConfigId;

use rand::Rng;

use eyre::{eyre, Context};
use std::io::{self, Cursor};
use std::collections::{HashMap, HashSet};

use reflection::PipelineDescription;

#[derive(Parser)]
pub struct Options {
    /// Location that files will be put in, by default working directory
    #[clap(action, default_value = "/")]
    pub target: String,

    /// Number of times for files to be generated and tested
    #[clap(long, action, default_value = "5")]
    pub repeat: u32,

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
    
    // Below are the options for data_race_generator that will be used in each loop
    // TODO: Check clap to see if there is a better way to lift these options
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
    pub constant_locs: u32
}

#[derive(Deserialize, Debug)]
pub struct Info{
    safe: Vec<u32>,
    locs_per_thread: u32,
    safe_constants: Vec<u32>,
    constant_locs: u32
}

pub fn run(options: Options) -> eyre::Result<()> {
    // 1) Construct a data race gen Options struct
    let mut iteration: u32 = 0;

    // 2) Loop and run for repeat times
    while options.inf_run || iteration < options.repeat {
        fs::create_dir_all(options.target.clone() + &"/".to_owned() + &iteration.to_string())?;
        let opt = data_race_generator::Options {
            seed: options.seed,
            debug: options.debug,
            log: options.log.clone(),
            workgroup_size: options.workgroup_size,
            racy_loc_pct: options.racy_loc_pct,
            racy_constant_loc_pct: options.racy_constant_loc_pct,
            racy_var_pct: options.racy_var_pct,
            num_lits: options.num_lits,
            stmts: options.stmts,
            vars: options.vars,
            locs_per_thread: options.locs_per_thread,
            constant_locs: options.constant_locs,
            output: options.target.clone() + &"/".to_owned() + &iteration.to_string() + &"/".to_owned() + &iteration.to_string() + &".wgsl".to_owned()
        };
        data_race_generator::run(opt)?;

        let configs = 
        if options.configs.len() == 0 {
            // Get defaults for run
            harness::default_configs()
        } else {
            options.configs.clone()
        };

        let folder_path = options.target.clone() + &"/".to_owned() + &iteration.to_string() + &"/".to_owned();

        let safe_wgsl = folder_path.clone() + &"safe_".to_owned() + &iteration.to_string() + &".wgsl".to_owned();
        let race_wgsl = folder_path.clone() + &"race_".to_owned() + &iteration.to_string() + &".wgsl".to_owned();

        let safe_json = folder_path.clone() + &"safe_".to_owned() + &iteration.to_string() + &".json".to_owned();
        let race_json = folder_path.clone() + &"race_".to_owned() + &iteration.to_string() + &".json".to_owned();

        // Get the size for input
        let input_size = ((options.workgroup_size * options.workgroups * options.locs_per_thread) + options.constant_locs) * 4; // Mult by 4 since u8

        // Lets randomize the inputs and write the json file for the runs
        let mut rng = rand::thread_rng();
        let random_data: Vec<u8> = (0..input_size).map(|_| rng.gen_range(0..u8::MAX)).collect();

        let input_json = json!({
            "0:0": random_data
        });

        let mut safe_data_out: Box<dyn io::Write> = Box::new(BufWriter::new(File::create(safe_json)?));
        let mut race_data_out: Box<dyn io::Write> = Box::new(BufWriter::new(File::create(race_json)?));
        println!("{:?}", input_json);

        writeln!(safe_data_out, "{input_json}")?;
        writeln!(race_data_out, "{input_json}")?;

        drop(safe_data_out);
        drop(race_data_out);

        let safe_shader = read_shader_from_path(&safe_wgsl)?;
        let race_shader = read_shader_from_path(&race_wgsl)?;

        // In practice I don't think these differ but for consistency we will do this for both
        let safe_input_data = read_input_data(&safe_wgsl, None)?; // None here to make the default input be used
        let race_input_data = read_input_data(&race_wgsl, None)?; // For now we will use defaults and later make input data

        let (safe_pipeline_desc, _) = reflect_shader(safe_shader.as_str(), safe_input_data);
        let (race_pipeline_desc, _) = reflect_shader(race_shader.as_str(), race_input_data);

        // TODO: Open info.json and extract data to compare in the future wrap as a struct and send it and outputs to a comparator crate for better comparison
        let info: Info = serde_json::from_reader(File::open(folder_path.clone() + &"info.json".to_owned())?)?;

        let mut is_okay = true;
        let mut bad_configs: Vec<ConfigId> = vec![];

        for config in configs {
            let safe_output = harness::execute_config(&safe_shader, options.workgroups, &safe_pipeline_desc, &config)?;
            let race_output = harness::execute_config(&race_shader, options.workgroups, &race_pipeline_desc, &config)?;
            
            let safe_array = u8s_to_u32s(&safe_output[0]);
            let race_array = u8s_to_u32s(&race_output[0]);
            
            let mut const_check: bool = true;
            let mut mem_check: bool = true;

            for const_index in info.safe_constants.clone() {
                let ind: usize = usize::try_from(const_index).unwrap();
                if safe_array[ind] != race_array[ind] {
                    const_check = false;
                    println!("{} has a mismatch in the constant region", ind.to_string().red());
                }
            }

            let num_threads = options.workgroups * options.workgroup_size;

            for thread_id in 0..num_threads {
                for offset in info.safe.clone() {
                    let ind: usize = usize::try_from(((thread_id * info.locs_per_thread) + offset) + info.constant_locs).unwrap();
                    if safe_array[ind] != race_array[ind] {
                        mem_check = false;
                        println!("{} has a mismatch in thread {}", ind.to_string().red(), thread_id.to_string().yellow());
                    }
                }
            }

            if !const_check || !mem_check {
                // This config leads to an issue
                bad_configs.push(config);
                is_okay = false;
            }
        }

        if is_okay {
            // Cleanup
            println!("{}", "All configs match".green());
            fs::remove_dir_all(&folder_path)?;
        } else {
            println!("{}", "Configs have a mismatch storing in error.out".red());
            let file = File::create(folder_path + &"/error.out")?;
            let mut writer = BufWriter::new(file);
            let out: String = bad_configs.into_iter().map(|i| i.to_string() + "\n").collect::<String>();

            writer.write_all(out.as_bytes())?;
        }
        
        if !options.inf_run {        
            iteration+=1;
        }
    }
        // 2a) update the output field of the Options struct with the loop file name

        // 2b) Run the generator

        // 2c) Generate the json file for input for now just 0s

        // 2d) Grab the harness configuration

        // 2e) Call the comparator crate (for now just put it here)

        // 2f) Do something if mismatch or match

        // 2g) If inf run is set reset the counter var of the loop to keep running
    Ok(())
}

// Below are harness functions that are copied here for ease of use. Reflect shader may be useful later to check
// types of the shader pipeline

fn read_shader_from_path(path: &str) -> eyre::Result<String> {
    let mut input: Box<dyn Read> = match path {
        "-" => Box::new(std::io::stdin()),
        path => Box::new(File::open(path)?),
    };

    let mut shader = String::new();
    input.read_to_string(&mut shader)?;

    Ok(shader)
}

fn read_input_data(
    shader: &str,
    input_data: Option<&str>,
) -> eyre::Result<HashMap<String, Vec<u8>>> {
    match input_data {
        Some(input_data) => {
            match serde_json::from_str(input_data)
                .wrap_err_with(|| eyre!("failed to parse input data"))
            {
                Ok(input_data) => Ok(input_data),
                Err(parse_err) => match File::open(input_data) {
                    Ok(file) => serde_json::from_reader(file)
                        .wrap_err_with(|| eyre!("failed to parse input data")),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Err(parse_err),
                    Err(e) => Err(e.into()),
                },
            }
        }
        None => {
            if shader != "-" {
                if let Some(path) = Path::new(shader).parent().map(|it| it.join("inputs.json")){
                    if path.exists() {
                        return Ok(serde_json::from_reader(File::open(path)?)?);
                    }
                }
                let path = Path::new(shader).with_extension("json");
                if path.exists() {
                    return Ok(serde_json::from_reader(File::open(path)?)?);
                }
            }
            Ok(Default::default())
        }
    }
}

fn reflect_shader(
    shader: &str,
    mut input_data: HashMap<String, Vec<u8>>,
) -> (PipelineDescription, Vec<common::Type>) {
    let module = parser::parse(shader);

    let (mut pipeline_desc, type_descs) = reflection::reflect(&module, |resource| {
            input_data.remove(&format!("{}:{}", resource.group, resource.binding))
    });

    let mut resource_vars = HashSet::new();

    for resource in &pipeline_desc.resources {
        resource_vars.insert(resource.name.clone());
    }

    utils::remove_accessed_vars(&mut resource_vars, &module);

    pipeline_desc
        .resources
        .retain(|resource| !resource_vars.contains(&resource.name));

    (pipeline_desc, type_descs)
}

fn u8s_to_u32s(from: &Vec<u8>) -> Vec<u32> {
    use byteorder::{LittleEndian, ReadBytesExt};
    let mut rdr = Cursor::new(from);
    let mut vec32: Vec<u32> = vec![];
    while let Ok(u) = rdr.read_u32::<LittleEndian>() {
        vec32.push(u);
    }
    vec32
}
