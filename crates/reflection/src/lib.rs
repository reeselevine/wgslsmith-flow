use ast::{Module, StorageClass, VarQualifier};
pub use types::{PipelineDescription, PipelineResource, ResourceData, ResourceKind};



fn update_size(value: &ast::DataType, init: &Option<Vec<u8>>) -> ast::DataType {
  match value {
    ast::DataType::Array(inner, size) => {
      ast::DataType::array (
      inner.as_ref().clone(),
      size.or(init.as_ref().map(|i| u32::try_from(i.len()/inner.size_bytes()).ok()).flatten()))
    },
    other => other.clone()
  }
}

pub fn reflect(
    module: &Module,
    mut init: impl FnMut(ResourceData<'_>) -> Option<Vec<u8>>,
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

            let init = buffer_init.map(|mut init| {
              init.resize(type_desc.buffer_size() as usize, 0);
              init
            });

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
