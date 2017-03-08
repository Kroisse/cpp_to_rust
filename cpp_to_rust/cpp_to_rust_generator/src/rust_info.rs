use cpp_ffi_data::CppAndFfiMethod;
use cpp_type::CppType;
use common::errors::{Result, unexpected};
use common::string_utils::JoinWithString;
use common::utils::MapIfOk;
use rust_type::{RustName, CompleteType, RustType, RustTypeIndirection, RustToCTypeConversion};
use cpp_method::CppMethodDoc;
use cpp_data::CppTypeDoc;
pub use serializable::{RustEnumValue, RustTypeWrapperKind, RustProcessedTypeInfo, RustExportInfo,
                       CppEnumValueDocItem, RustQtSlotWrapper};
use cpp_ffi_data::IndirectionChange;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustMethodDocItem {
  pub doc: Option<CppMethodDoc>,
  pub rust_fns: Vec<String>,
  pub cpp_fn: String,
  pub rust_cross_references: Vec<RustCrossReference>,
}



#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RustMethodScope {
  Impl { target_type: RustType },
  TraitImpl,
  Free,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustMethodArgument {
  pub argument_type: CompleteType,
  pub name: String,
  pub ffi_index: Option<i32>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustMethodArgumentsVariant {
  pub arguments: Vec<RustMethodArgument>,
  pub cpp_method: CppAndFfiMethod,
  pub return_type_ffi_index: Option<i32>,
  pub return_type: CompleteType,
}

// impl RustMethodArgumentsVariant {
//  pub fn has_unportable_arg_types(&self) -> bool {
//    self.arguments.iter().any(|arg| arg.argument_type.cpp_type.is_platform_dependent())
//  }
// }

#[derive(Debug, PartialEq, Eq, Clone)]
#[allow(dead_code)]
pub enum RustMethodArguments {
  SingleVariant(RustMethodArgumentsVariant),
  MultipleVariants {
    params_trait_name: String,
    params_trait_lifetime: Option<String>,
    params_trait_return_type: Option<RustType>,
    shared_arguments: Vec<RustMethodArgument>,
    variant_argument_name: String,
    cpp_method_name: String,
  },
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustMethod {
  pub scope: RustMethodScope,
  pub is_unsafe: bool,
  pub name: RustName,
  pub arguments: RustMethodArguments,
  pub docs: Vec<RustMethodDocItem>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustSingleMethod {
  pub scope: RustMethodScope,
  pub is_unsafe: bool,
  pub name: RustName,
  pub arguments: RustMethodArgumentsVariant,
  pub doc: Option<RustMethodDocItem>,
}


#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum RustMethodSelfArgKind {
  Static,
  ConstRef,
  MutRef,
  Value,
}

fn detect_self_arg_kind(args: &[RustMethodArgument]) -> Result<RustMethodSelfArgKind> {
  Ok(if let Some(arg) = args.get(0) {
    if arg.name == "self" {
      if let RustType::Common { ref indirection, ref is_const, .. } = arg.argument_type
        .rust_api_type {
        match *indirection {
          RustTypeIndirection::Ref { .. } => {
            if *is_const {
              RustMethodSelfArgKind::ConstRef
            } else {
              RustMethodSelfArgKind::MutRef
            }
          }
          RustTypeIndirection::None => RustMethodSelfArgKind::Value,
          _ => return Err(unexpected("invalid self argument type").into()),
        }
      } else {
        return Err(unexpected("invalid self argument type").into());
      }
    } else {
      RustMethodSelfArgKind::Static
    }
  } else {
    RustMethodSelfArgKind::Static
  })
}

impl RustMethod {
  //  pub fn self_arg_kind(&self) -> Result<RustMethodSelfArgKind> {
  //    let args = match self.arguments {
  //      RustMethodArguments::SingleVariant(ref var) => &var.arguments,
  //      RustMethodArguments::MultipleVariants { ref shared_arguments, .. } => shared_arguments,
  //    };
  //    detect_self_arg_kind(args)
  //  }

  #[allow(dead_code)]
  pub fn cpp_cross_references(&self) -> Vec<String> {
    let mut r = Vec::new();
    for doc in &self.docs {
      if let Some(ref doc) = doc.doc {
        r.append(&mut doc.cross_references.clone());
      }
    }
    r
  }

  #[allow(dead_code)]
  pub fn add_rust_cross_references(&mut self, table: HashMap<String, RustCrossReference>) {
    for doc in &mut self.docs {
      let mut result = Vec::new();
      if let Some(ref doc) = doc.doc {
        for reference in &doc.cross_references {
          if let Some(r) = table.get(reference) {
            result.push(r.clone());
          }
        }
      }
      doc.rust_cross_references = result;
    }
  }
}

impl RustSingleMethod {
  pub fn to_rust_method(&self) -> RustMethod {
    RustMethod {
      name: self.name.clone(),
      arguments: RustMethodArguments::SingleVariant(self.arguments.clone()),
      docs: if let Some(ref doc) = self.doc {
        vec![doc.clone()]
      } else {
        Vec::new()
      },
      is_unsafe: self.is_unsafe,
      scope: self.scope.clone(),
    }
  }

  pub fn self_arg_kind(&self) -> Result<RustMethodSelfArgKind> {
    detect_self_arg_kind(&self.arguments.arguments)
  }

  pub fn can_be_overloaded_with(&self, other_method: &RustSingleMethod) -> Result<bool> {
    // println!("can_be_overloaded_with {:?} | {:?}", self, other_method);
    if self.self_arg_kind()? != other_method.self_arg_kind()? {
      // println!("false1");
      return Ok(false);
    }
    if self.arguments.arguments.len() == other_method.arguments.arguments.len() {
      if self.arguments
        .arguments
        .iter()
        .zip(other_method.arguments.arguments.iter())
        .all(|(arg1, arg2)| {
          arg1.argument_type.cpp_type.can_be_the_same_as(&arg2.argument_type.cpp_type) &&
          !(arg1.name == "allocation_place_marker" && arg2.name == "allocation_place_marker" &&
            arg1 != arg2)
        }) {
        // println!("false2");
        return Ok(false);
      }
    }
    // println!("true0");
    Ok(true)
  }

  pub fn name_suffix(&self,
                     caption_strategy: &RustMethodCaptionStrategy,
                     all_self_args: &HashSet<RustMethodSelfArgKind>,
                     index: usize)
                     -> Result<Option<String>> {
    Ok({
      let self_arg_kind = self.self_arg_kind()?;
      let self_arg_kind_caption = if all_self_args.len() == 1 ||
                                     self_arg_kind == RustMethodSelfArgKind::ConstRef {
        None
      } else if self_arg_kind == RustMethodSelfArgKind::Static {
        Some("static")
      } else if self_arg_kind == RustMethodSelfArgKind::MutRef {
        if all_self_args.contains(&RustMethodSelfArgKind::ConstRef) {
          Some("mut")
        } else {
          None
        }
      } else {
        return Err("unsupported self arg kinds combination".into());
      };
      let other_caption = match *caption_strategy {
        RustMethodCaptionStrategy::NoArgs => None,
        RustMethodCaptionStrategy::Index => Some(index.to_string()),
        RustMethodCaptionStrategy::ArgNames => {
          if self.arguments.arguments.is_empty() {
            Some("no_args".to_string())
          } else {
            Some(self.arguments.arguments.iter().map(|a| &a.name).join("_"))
          }
        }
        RustMethodCaptionStrategy::ArgTypes => {
          if self.arguments.arguments.is_empty() {
            Some("no_args".to_string())
          } else {
            Some(self.arguments
              .arguments
              .iter()
              .map_if_ok(|t| t.argument_type.rust_api_type.caption())?
              .join("_"))
          }
        }
      };
      let mut key_caption_items = Vec::new();
      if let Some(c) = self_arg_kind_caption {
        key_caption_items.push(c.to_string());
      }
      if let Some(c) = other_caption {
        key_caption_items.push(c);
      }
      if key_caption_items.is_empty() {
        None
      } else {
        Some(key_caption_items.join("_"))
      }
    })
  }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TraitImplExtra {
  CppDeletable { deleter_name: String },
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TraitImpl {
  pub target_type: RustType,
  pub trait_type: RustType,
  pub extra: Option<TraitImplExtra>,
  pub methods: Vec<RustMethod>,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RustCrossReferenceKind {
  Method { scope: RustMethodScope },
  Type,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustCrossReference {
  name: RustName,
  kind: RustCrossReferenceKind,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RustQtReceiverType {
  Signal,
  Slot,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustQtReceiverDeclaration {
  pub type_name: String,
  pub method_name: String,
  pub receiver_type: RustQtReceiverType,
  pub receiver_id: String,
  pub arguments: Vec<RustType>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RustTypeDeclarationKind {
  CppTypeWrapper {
    kind: RustTypeWrapperKind,
    cpp_type_name: String,
    cpp_template_arguments: Option<Vec<CppType>>,
    cpp_doc: Option<CppTypeDoc>,
    rust_cross_references: Vec<RustCrossReference>,
    methods: Vec<RustMethod>,
    trait_impls: Vec<TraitImpl>,
    qt_receivers: Vec<RustQtReceiverDeclaration>,
  },
  MethodParametersTrait {
    lifetime: Option<String>,
    shared_arguments: Vec<RustMethodArgument>,
    return_type: Option<RustType>,
    impls: Vec<RustMethodArgumentsVariant>,
    method_scope: RustMethodScope,
    method_name: RustName,
    is_unsafe: bool,
  },
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustTypeDeclaration {
  pub is_public: bool,
  pub name: RustName,
  pub kind: RustTypeDeclarationKind,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RustModule {
  pub name: String,
  pub types: Vec<RustTypeDeclaration>,
  pub functions: Vec<RustMethod>,
  pub trait_impls: Vec<TraitImpl>,
  pub submodules: Vec<RustModule>,
}

#[derive(Debug, Clone)]
pub struct DependencyInfo {
  pub rust_export_info: RustExportInfo,
  pub cache_path: PathBuf,
}


pub enum RustMethodCaptionStrategy {
  NoArgs,
  ArgTypes,
  ArgNames,
  Index,
}
impl RustMethodCaptionStrategy {
  pub fn all() -> &'static [RustMethodCaptionStrategy] {
    use self::RustMethodCaptionStrategy::*;
    const LIST: &'static [RustMethodCaptionStrategy] = &[NoArgs, ArgTypes, ArgNames, Index];
    return LIST;
  }
}

pub fn allocation_place_marker(marker_name: &'static str) -> Result<RustMethodArgument> {
  Ok(RustMethodArgument {
    name: "allocation_place_marker".to_string(),
    ffi_index: None,
    argument_type: CompleteType {
      cpp_type: CppType::void(),
      cpp_ffi_type: CppType::void(),
      cpp_to_ffi_conversion: IndirectionChange::NoChange,
      rust_ffi_type: RustType::Void,
      rust_api_type: RustType::Common {
        base: RustName::new(vec!["cpp_utils".to_string(), marker_name.to_string()])?,
        generic_arguments: None,
        is_const: false,
        is_const2: false,
        indirection: RustTypeIndirection::None,
      },
      rust_api_to_c_conversion: RustToCTypeConversion::None,
    },
  })
}