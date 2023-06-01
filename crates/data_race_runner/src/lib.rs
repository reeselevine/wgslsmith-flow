pub mod cli;

use std::{collections::HashMap, io::Cursor};
use colored::Colorize;

use data_race_generator::DataRaceInfo;
use reflection::PipelineDescription;
use types::ConfigId;

pub struct ExecOptions {
  configs: Vec<ConfigId>,
  workgroups: u32,
  workgroup_size: u32,
  reps: u32
}

pub fn execute(
    racy_shader: String,
    safe_shader: String,
    data_race_info: DataRaceInfo,
    input_data: HashMap<String, Vec<u8>>,
    exec_options: ExecOptions
) {
    let safe_pipeline_desc = reflect_shader(safe_shader.as_str(), &input_data);
    let race_pipeline_desc = reflect_shader(racy_shader.as_str(), &input_data);

    let mut is_okay = true;
    let mut error_out: Vec<String> = vec![];

    for config in exec_options.configs {
        error_out.push("Config: ".to_owned() + &config.to_string() + &"\n\n".to_string());
        for rep in 0..exec_options.reps {
            error_out.push("Rep: ".to_owned() + &rep.to_string() + &"\n".to_string());
            let safe_output = harness::execute_config(
                &safe_shader,
                exec_options.workgroups,
                &safe_pipeline_desc,
                &config,
            ).unwrap();
            let race_output = harness::execute_config(
                &racy_shader,
                exec_options.workgroups,
                &race_pipeline_desc,
                &config,
            ).unwrap();

            let safe_array = u8s_to_u32s(&safe_output[0]);
            let race_array = u8s_to_u32s(&race_output[0]);

            for const_index in data_race_info.safe_constants.clone() {
                let ind: usize = usize::try_from(const_index).unwrap();
                if safe_array[ind] != race_array[ind] {
                    is_okay = false;
                    error_out.push(
                        "Constant mismatch: Index ".to_owned()
                            + &ind.to_string()
                            + &" has a mismatch in the constant region.\n".to_string(),
                    );
                    println!(
                        "{} has a mismatch in the constant region",
                        ind.to_string().red()
                    );
                }
            }

            let num_threads = exec_options.workgroups * exec_options.workgroup_size;

            for thread_id in 0..num_threads {
                for offset in data_race_info.safe.clone() {
                    let ind: usize = usize::try_from(
                        ((thread_id * data_race_info.locs_per_thread) + offset) + data_race_info.constant_locs,
                    )
                    .unwrap();
                    if safe_array[ind] != race_array[ind] {
                        is_okay = false;
                        error_out.push(
                            "Memory mismatch: Index ".to_owned()
                                + &ind.to_string()
                                + &" has a mismatch in thread_id ".to_string()
                                + &thread_id.to_string()
                                + &"\n".to_string(),
                        );
                        println!(
                            "{} has a mismatch in thread {}",
                            ind.to_string().red(),
                            thread_id.to_string().yellow()
                        );
                    }
                }
            }
        }
        error_out.push("\n".to_string());
    }

    if is_okay {
        println!("{}", "All configs match".green());
    } else {
        println!("{}", "Configs have a mismatch".red());
    }
}

fn reflect_shader(
    shader: &str,
    input_data: &HashMap<String, Vec<u8>>,
) -> PipelineDescription {
    let module = parser::parse(shader);

    let (pipeline_desc, _) = reflection::reflect(&module, |resource| {
            input_data.get(&format!("{}:{}", resource.group, resource.binding)).cloned()
    });
    pipeline_desc
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

