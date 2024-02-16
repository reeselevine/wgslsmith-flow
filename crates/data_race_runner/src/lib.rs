pub mod cli;

use std::{collections::HashMap, io::Cursor};

use data_race_generator::{DataRaceInfo, RaceValueStrategy};
use reflection::PipelineDescription;
use reflection_types::BufferInitInfo;
use serde::Serialize;
use types::ConfigId;

pub struct ExecOptions {
    pub configs: Vec<ConfigId>,
    pub workgroups: u32,
    pub workgroup_size: u32,
    pub reps: u32,
}

#[derive(Debug, Serialize)]
pub enum Expected {
    Value(u32),
    Strategy(RaceValueStrategy),
}

#[derive(Debug, Serialize)]
pub enum MismatchType {
  ConstantLocation,
  SafeLocation,
  UninitializedVar
}

#[derive(Debug, Serialize)]
pub struct Mismatch {
    pub config: ConfigId,
    pub rep: u32,
    pub mismatch_type: MismatchType,
    pub thread: Option<u32>,
    pub index: u32,
    pub expected: Expected,
    pub actual: u32,
}

pub fn execute(
    racy_shader: String,
    safe_shader: String,
    data_race_info: &DataRaceInfo,
    input_data: &HashMap<String, BufferInitInfo>,
    exec_options: ExecOptions,
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
            )
            .unwrap();
            let race_output = harness::execute_config(
                &racy_shader,
                exec_options.workgroups,
                &race_pipeline_desc,
                &config,
            )
            .unwrap();

            let safe_array = u8s_to_u32s(&safe_output[0]);
            let race_array = u8s_to_u32s(&race_output[0]);

            let safe_uninit_array = u8s_to_u32s(&safe_output[1]);
            let race_uninit_array = u8s_to_u32s(&race_output[1]);

            for const_index in 0..data_race_info.constant_locs {
                let index: usize = usize::try_from(const_index).unwrap();
                if data_race_info.safe_constants.contains(&const_index) {
                    if safe_array[index] != race_array[index] {
                        mismatches.push(Mismatch {
                            config: config.clone(),
                            rep,
                            mismatch_type: MismatchType::ConstantLocation,
                            thread: None,
                            index: u32::try_from(index).unwrap(),
                            expected: Expected::Value(safe_array[index]),
                            actual: race_array[index],
                        });
                    }
                }
                match data_race_info.race_val_strat {
                    Some(RaceValueStrategy::Even) => {
                        if race_array[index] % 2 != 0 {
                            mismatches.push(Mismatch {
                                config: config.clone(),
                                rep,
                                mismatch_type: MismatchType::ConstantLocation,
                                thread: None,
                                index: u32::try_from(index).unwrap(),
                                expected: Expected::Strategy(RaceValueStrategy::Even),
                                actual: race_array[index],
                            });
                        }
                    }
                    None => {}
                }
            }

            let num_threads = exec_options.workgroups * exec_options.workgroup_size;

            for thread_id in 0..num_threads {
                for offset in 0..data_race_info.num_uninit_vars {
                  let ind: usize = usize::try_from(
                    thread_id * data_race_info.num_uninit_vars + offset).unwrap();
                  if safe_uninit_array[ind] != 0 {
                    mismatches.push(Mismatch {
                      config: config.clone(),
                      rep,
                      mismatch_type: MismatchType::UninitializedVar,
                      thread: Some(thread_id),
                      index: u32::try_from(ind).unwrap(),
                      expected: Expected::Value(0),
                      actual: safe_uninit_array[ind],
                    });
                  }
                  if race_uninit_array[ind] != 0 {
                    mismatches.push(Mismatch {
                      config: config.clone(),
                      rep,
                      mismatch_type: MismatchType::UninitializedVar,
                      thread: Some(thread_id),
                      index: u32::try_from(ind).unwrap(),
                      expected: Expected::Value(0),
                      actual: race_uninit_array[ind],
                    });
                  }
                }

                for offset in 0..data_race_info.locs_per_thread {
                    let ind: usize = usize::try_from(
                        ((thread_id * data_race_info.locs_per_thread) + offset)
                            + data_race_info.constant_locs,
                    )
                    .unwrap();
                    if data_race_info.safe.contains(&offset) {
                        if safe_array[ind] != race_array[ind] {
                            mismatches.push(Mismatch {
                                config: config.clone(),
                                rep,
                                mismatch_type: MismatchType::SafeLocation,
                                thread: Some(thread_id),
                                index: u32::try_from(ind).unwrap(),
                                expected: Expected::Value(safe_array[ind]),
                                actual: race_array[ind],
                            });
                        }
                    }
                    match data_race_info.race_val_strat {
                        Some(RaceValueStrategy::Even) => {
                            if race_array[ind] % 2 != 0 {
                                mismatches.push(Mismatch {
                                    config: config.clone(),
                                    rep,
                                    mismatch_type: MismatchType::SafeLocation,
                                    thread: Some(thread_id),
                                    index: u32::try_from(ind).unwrap(),
                                    expected: Expected::Strategy(RaceValueStrategy::Even),
                                    actual: race_array[ind],
                                });
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }
    mismatches
}

fn reflect_shader(shader: &str, input_data: &HashMap<String, BufferInitInfo>) -> PipelineDescription {
    let module = parser::parse(shader);

    let (pipeline_desc, _) = reflection::reflect(&module, |resource| {
        input_data
            .get(&format!("{}:{}", resource.group, resource.binding))
            .cloned()
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
