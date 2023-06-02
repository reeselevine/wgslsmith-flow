pub mod cli;

use std::{collections::HashMap, io::Cursor};

use data_race_generator::DataRaceInfo;
use reflection::PipelineDescription;
use serde::Serialize;
use types::ConfigId;

pub struct ExecOptions {
  pub configs: Vec<ConfigId>,
  pub workgroups: u32,
  pub workgroup_size: u32,
  pub reps: u32
}

#[derive(Debug, Serialize)]
pub struct Mismatch {
  pub config: ConfigId,
  pub rep: u32,
  // if thread is none than this is a constant location mismatch
  pub thread: Option<u32>,
  pub index: u32,
  pub expected: u32,
  pub actual: u32
}

pub fn execute(
    racy_shader: String,
    safe_shader: String,
    data_race_info: &DataRaceInfo,
    input_data: &HashMap<String, Vec<u8>>,
    exec_options: ExecOptions
) -> Vec<Mismatch> {
    let safe_pipeline_desc = reflect_shader(safe_shader.as_str(), &input_data);
    let race_pipeline_desc = reflect_shader(racy_shader.as_str(), &input_data);
    let mut mismatches = vec![];
    for config in exec_options.configs {
        for rep in 0..exec_options.reps {
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
                let index: usize = usize::try_from(const_index).unwrap();
                if safe_array[index] != race_array[index] {
                    mismatches.push(Mismatch {
                      config: config.clone(), 
                      rep, 
                      thread: None, 
                      index: u32::try_from(index).unwrap(),
                      expected: safe_array[index],
                      actual: race_array[index]
                    });
                }
            }

            let num_threads = exec_options.workgroups * exec_options.workgroup_size;

            for thread_id in 0..num_threads {
                for offset in data_race_info.safe.clone() {
                    let ind: usize = usize::try_from(
                        ((thread_id * data_race_info.locs_per_thread) + offset) + data_race_info.constant_locs).unwrap();
                    if safe_array[ind] != race_array[ind] {
                        mismatches.push(Mismatch { 
                          config: config.clone(), 
                          rep, 
                          thread: Some(thread_id), 
                          index: u32::try_from(ind).unwrap(),
                          expected: safe_array[ind],
                          actual: race_array[ind]
                        });
                    }
                }
            }
        }
    }
    mismatches
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

