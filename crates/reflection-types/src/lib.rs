use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub struct ResourceData<'a> {
    pub name: &'a str,
    pub group: u32,
    pub binding: u32,
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct PipelineDescription {
    pub resources: Vec<PipelineResource>,
}

#[derive(Clone, Debug, Decode, Encode, PartialEq, Eq)]
pub enum ResourceKind {
    StorageBuffer,
    UniformBuffer,
}

#[derive(Clone, Debug, Decode, Encode, Serialize, Deserialize)]
pub enum BufferInitInfo {
  Data { data: Vec<u8> },
  Size { size: u32 }
}

impl BufferInitInfo {
  pub fn size_in_elem(&self, bytes_per_elem: usize) -> u32 {
    match self {
      BufferInitInfo::Data { data } => u32::try_from(data.len() / bytes_per_elem ).unwrap(),
      BufferInitInfo::Size { size } => size / u32::try_from(bytes_per_elem).unwrap()
    }
  }
}

#[derive(Clone, Debug, Decode, Encode)]
pub struct PipelineResource {
    pub name: String,
    pub kind: ResourceKind,
    pub group: u32,
    pub binding: u32,
    pub init: Option<Vec<u8>>,
    pub size: u32,
}
