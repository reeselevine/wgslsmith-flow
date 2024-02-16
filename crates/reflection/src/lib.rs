use ast::{Module, StorageClass, VarQualifier};
use types::BufferInitInfo;
pub use types::{PipelineDescription, PipelineResource, ResourceData, ResourceKind};



// If the type is a runtime array, we populate the size from the buffer init info, which is passed as either a vector
// of bytes or a size in bytes. The final size of the array is in elements, so bytes/bytes_per_element.
fn update_size(value: &ast::DataType, init_data: &Option<BufferInitInfo>) -> ast::DataType {
  match value {
    ast::DataType::Array(inner, size) => {
      ast::DataType::array (
      inner.as_ref().clone(),
      size.or(init_data.as_ref().map(|data| data.size_in_elem(inner.size_bytes()))))
    },
    other => other.clone()
  }
}

pub fn reflect(
    module: &Module,
    mut init: impl FnMut(ResourceData<'_>) -> Option<BufferInitInfo>,
) -> (PipelineDescription, Vec<common::Type>) {
    let mut resources = vec![];
    let mut types = vec![];

    for var in &module.vars {
        if let Some(VarQualifier { storage_class, .. }) = &var.qualifier {
            let kind = match storage_class {
                StorageClass::Uniform => ResourceKind::UniformBuffer,
                StorageClass::Storage => ResourceKind::StorageBuffer,
                _ => continue,
            };

            let group = var
                .group_index()
                .expect("resource variable must have group attribute");

            let binding = var
                .binding_index()
                .expect("resource variable must have binding attribute");

            let buffer_init = init(ResourceData {
                name: &var.name,
                group,
                binding,
            });

            let data_type = update_size(&var.data_type, &buffer_init);

            let type_desc =
                common::Type::try_from(&data_type).expect("invalid type for pipeline resource");

            let init = buffer_init.map(|init| {
              match init {
                BufferInitInfo::Data { mut data } => {
                  data.resize(type_desc.buffer_size() as usize, 0);
                  Some(data)
                }
                BufferInitInfo::Size { size: _ } => None
              }
            }).flatten();

            resources.push(PipelineResource {
                name: var.name.clone(),
                kind,
                group,
                binding,
                init,
                size: type_desc.size(),
            });

            types.push(type_desc);
        }
    }

    (PipelineDescription { resources }, types)
}
