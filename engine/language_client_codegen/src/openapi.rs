use std::path::PathBuf;

use anyhow::Result;
use indexmap::IndexMap;
use internal_baml_core::{
    configuration::GeneratorDefaultClientMode,
    ir::{repr::IntermediateRepr, FieldType, IRHelper},
};
use serde::Serialize;

use crate::dir_writer::{FileCollector, LanguageFeatures};

#[derive(Default)]
pub(super) struct OpenApiLanguageFeatures {}

impl LanguageFeatures for OpenApiLanguageFeatures {
    const CONTENT_PREFIX: &'static str = r#"
###############################################################################
#
#  Welcome to Baml! To use this generated code, please run the following:
#
#  $ openapi-generator generate -i openapi.yaml -g <language> -o <output_dir>
#
###############################################################################

# This file was generated by BAML: please do not edit it. Instead, edit the
# BAML files and re-generate this code.

        "#;
}

#[derive(Serialize)]
struct OpenApiSchema {
    paths: IndexMap<String, IndexMap<String, OpenApiMethodDef>>,
    schemas: IndexMap<String, OpenApiClassDef>,
}

#[derive(Serialize)]
struct OpenApiMethodDef {
    requestBody: String,
    // requestBody:
    // content:
    //   application/json:
    //     schema:
    //       $ref: '#/components/schemas/Order'
    // description: order placed for purchasing the pet
    // required: true
}

#[derive(Serialize)]
struct OpenApiClassDef {
    name: String,
    title: String,
    description: String,
    r#type: String,
    properties: IndexMap<String, OpenApiTypeDef>,
}

#[derive(Serialize)]
struct OpenApiTypeDef {
    r#type: String,
    format: String,
}

pub(crate) fn generate(
    ir: &IntermediateRepr,
    generator: &crate::GeneratorArgs,
) -> Result<IndexMap<PathBuf, String>> {
    let mut collector = FileCollector::<OpenApiLanguageFeatures>::new();

    let schema = OpenApiSchema {
        paths: Default::default(),
        schemas: Default::default(),
    };

    collector.add_file("openapi.yaml", serde_yaml::to_string(&schema)?);

    collector.commit(&generator.output_dir())
}
