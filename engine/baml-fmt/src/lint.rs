#![allow(dead_code)]

use internal_baml_jinja::{
    ChatMessagePart, RenderContext, RenderContext_Client, RenderedChatMessage,
    TemplateStringMacro,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
mod jsonschema;

use jsonschema::WithJsonSchema;

use baml_lib::{
    internal_baml_diagnostics::{DatamodelError, DatamodelWarning},
    internal_baml_parser_database::{
        walkers::{FunctionWalker, VariantWalker},
        PromptAst, WithSerialize,
    },
    internal_baml_schema_ast::ast::{self, WithIdentifier, WithName, WithSpan},
    SourceFile, ValidatedSchema,
};

#[derive(Serialize)]
pub struct StringSpan {
    value: String,
    start: usize,
    end: usize,
    source_file: String,
}

impl StringSpan {
    pub fn new<S: Into<String>>(
        value: S,
        span: &baml_lib::internal_baml_diagnostics::Span,
    ) -> Self {
        Self {
            value: value.into(),
            start: span.start,
            end: span.end,
            source_file: span.file.path(),
        }
    }
}

#[derive(Serialize)]
pub struct MiniError {
    start: usize,
    end: usize,
    text: String,
    is_warning: bool,
    source_file: String,
}

#[derive(Deserialize)]
struct File {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct Input {
    root_path: String,
    files: Vec<File>,
    #[serde(default = "HashMap::new")]
    selected_tests: HashMap<String, String>,
}

pub(crate) fn run(input: &str) -> String {
    let input: Input = serde_json::from_str(input).expect("Failed to parse input");

    let files: Vec<SourceFile> = input
        .files
        .into_iter()
        .map(|file| SourceFile::new_allocated(file.path.into(), Arc::from(file.content)))
        .collect();

    let path = PathBuf::from(input.root_path);
    let schema = baml_lib::validate(&path, files);
    let diagnostics = &schema.diagnostics;

    let mut mini_errors: Vec<MiniError> = diagnostics
        .warnings()
        .iter()
        .map(|warn: &DatamodelWarning| MiniError {
            start: warn.span().start,
            end: warn.span().end,
            text: warn.message().to_owned(),
            is_warning: true,
            source_file: warn.span().file.path(),
        })
        .collect();

    if diagnostics.has_errors() {
        mini_errors.extend(
            diagnostics
                .errors()
                .iter()
                .map(|err: &DatamodelError| MiniError {
                    start: err.span().start,
                    end: err.span().end,
                    text: err.message().to_string(),
                    is_warning: false,
                    source_file: err.span().file.path(),
                }),
        );

        return print_diagnostics(mini_errors, None);
    }

    let template_string_macros = schema
        .db
        .walk_templates()
        .map(|w| TemplateStringMacro {
            name: w.name().to_string(),
            args: w.ast_node().input().map_or(vec![], |args| match args {
                ast::FunctionArgs::Named(list) => list
                    .args
                    .iter()
                    .map(|(idn, _)| (idn.name().to_string(), "<not set>".into()))
                    .collect(),
                _ => vec![],
            }),
            template: w.template_string().to_string(),
        })
        .collect::<Vec<_>>();

    let response = json!({
        "enums": schema.db.walk_enums().map(|e| json!({
            "name": StringSpan::new(e.name(), e.identifier().span()),
            "jsonSchema": e.json_schema(),
        })).collect::<Vec<_>>(),
        "classes": schema.db.walk_classes().map(|c| json!({
            "name": StringSpan::new(c.name(), c.identifier().span()),
            "jsonSchema": c.json_schema(),
        })).collect::<Vec<_>>(),
        "clients": schema.db.walk_clients().map(|c| json!({
            "name": StringSpan::new(c.name(), c.identifier().span()),
        })).collect::<Vec<_>>(),
        "functions": std::iter::empty()
            .chain(schema.db.walk_old_functions().map(|f| serialize_function(&schema, f, SFunctionSyntax::Version1, &template_string_macros, input.selected_tests.get(f.name()))))
            .chain(schema.db.walk_new_functions().map(|f| serialize_function(&schema, f, SFunctionSyntax::Version2, &template_string_macros, input.selected_tests.get(f.name()))))
            .collect::<Vec<_>>(),
    });

    print_diagnostics(mini_errors, Some(response))
}

// keep in sync with typescript/common/src/parser_db.ts
#[derive(Serialize)]
struct Span {
    start: usize,
    end: usize,
    source_file: String,
}

impl From<&baml_lib::internal_baml_diagnostics::Span> for Span {
    fn from(span: &baml_lib::internal_baml_diagnostics::Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
            source_file: span.file.path(),
        }
    }
}

#[derive(Serialize)]
enum SFunctionSyntax {
    Version1, // "impl<llm, ClassifyResume>"
    Version2, // functions and impls are collapsed into a single function Name(args) -> Output {...}
}

#[derive(Serialize)]
struct SFunction {
    name: StringSpan,
    input: serde_json::Value,
    output: serde_json::Value,
    test_cases: Vec<serde_json::Value>,
    impls: Vec<Impl>,
    syntax: SFunctionSyntax,
}

fn serialize_function(
    schema: &ValidatedSchema,
    func: FunctionWalker,
    syntax: SFunctionSyntax,
    template_string_macros: &[TemplateStringMacro],
    selected_test_name: Option<&String>,
) -> SFunction {
    SFunction {
        name: StringSpan::new(func.name(), func.identifier().span()),
        input: match func.ast_function().input() {
            ast::FunctionArgs::Named(arg_list) => json!({
                "arg_type": "named",
                "values": arg_list.args.iter().map(
                    |(id, arg)| json!({
                        "name": StringSpan::new(id.name(), id.span()),
                        "type": format!("{}", arg.field_type),
                        "jsonSchema": arg.field_type.json_schema()

                    })
                ).collect::<Vec<_>>(),
            }),
            ast::FunctionArgs::Unnamed(arg) => json!({
                "arg_type": "positional",
                "type": format!("{}", arg.field_type),
                "jsonSchema": arg.field_type.json_schema()
            }),
        },
        output: match func.ast_function().output() {
            ast::FunctionArgs::Named(arg_list) => json!({
                "arg_type": "named",
                "values": arg_list.args.iter().map(
                    |(id, arg)| json!({
                        "name": StringSpan::new(id.name(), id.span()),
                        "type": format!("{}", arg.field_type),
                        "jsonSchema": arg.field_type.json_schema()
                    })
                ).collect::<Vec<_>>(),
            }),
            ast::FunctionArgs::Unnamed(arg) => json!({
                "arg_type": "positional",
                "type": format!("{}", arg.field_type),
                "jsonSchema": arg.field_type.json_schema()
            }),
        },
        test_cases: func
            .walk_tests()
            .map(|t| {
                let props = t.test_case();
                json!({
                    "name": StringSpan::new(t.name(), t.identifier().span()),
                    // DO NOT LAND
                    "content": serde_json::from_str(&props.content.to_string()).unwrap_or(serde_json::Value::Null),
                })
            })
            .collect::<Vec<_>>(),
        impls: serialize_impls(schema, func, template_string_macros, selected_test_name),
        syntax,
    }
}

// keep in sync with typescript/common/src/parser_db.ts
#[derive(Serialize)]
#[serde(tag = "type")] // JSON is { "type": "completion", "completion": "..." }
enum PromptPreview {
    Completion {
        completion: String,
        test_case: Option<String>,
    },
    Chat {
        chat: Vec<RenderedChatMessage>,
        test_case: Option<String>,
    },
    Error {
        error: String,
        test_case: Option<String>,
    },
}

#[derive(Serialize)]
struct ImplClient {
    identifier: StringSpan,
    provider: String,
    model: Option<String>,
}

// keep in sync with typescript/common/src/parser_db.ts
#[derive(Serialize)]
struct Impl {
    name: StringSpan,
    prompt_key: Span,
    prompt: PromptPreview,
    client: ImplClient,
    client_list: Vec<String>,
    input_replacers: Vec<(String, String)>,
    output_replacers: Vec<(String, String)>,
}

fn apply_replacers(variant: VariantWalker, mut content: String) -> String {
    let (input_replacers, output_replacers, _) = &variant.properties().replacers;
    for (input_var, input_replacement) in input_replacers {
        content = content.replace(&input_var.key(), &format!("{{{input_replacement}}}"));
    }
    for (output_var, output_replacement) in output_replacers {
        content = content.replace(&output_var.key(), &output_replacement.to_string());
    }
    content
}

/// Returns details about the selected non-strategy client which is used in the function's `client`.
///
/// If no client is specified, or none matches the filter, the first non-strategy client is returned.
///
/// The returned span always points to the `client FooClient` reference in the function body.
fn choose_client(
    schema: &ValidatedSchema,
    func: FunctionWalker,
    client_filter: Option<String>,
) -> anyhow::Result<(ImplClient, RenderContext_Client, Vec<String>)> {
    let (client_or_strategy, client_span) = func.metadata().client.as_ref().ok_or(
        anyhow::anyhow!("function {} does not have a client", func.name()),
    )?;
    let clients = schema
        .db
        .find_client(client_or_strategy)
        .ok_or(anyhow::anyhow!("client {} not found", client_or_strategy))?
        .flat_clients();

    let client = match clients
        .iter()
        .find(|c| Some(c.name()) == client_filter.as_deref())
    {
        Some(c) => c,
        None => clients.first().ok_or(anyhow::anyhow!(
            "failed to resolve client {}",
            client_or_strategy
        ))?,
    };

    Ok((
        ImplClient {
            identifier: StringSpan::new(
                if client_or_strategy == client.name() {
                    client.name().to_string()
                } else {
                    format!("{} (via {})", client.name(), client_or_strategy)
                },
                client_span,
            ),
            provider: client.provider().to_string(),
            model: client.model().map(|m| m.to_string()),
        },
        RenderContext_Client {
            name: client.name().to_string(),
            provider: client.properties().provider.0.clone(),
        },
        clients.iter().map(|c| c.name().to_string()).collect(),
    ))
}

fn serialize_impls(
    schema: &ValidatedSchema,
    func: FunctionWalker,
    _template_string_macros: &[TemplateStringMacro],
    selected_test_name: Option<&String>,
) -> Vec<Impl> {
    if func.is_old_function() {
        func.walk_variants()
            .map(|i| {
                let props = i.properties();
                let client = schema.db.find_client(&props.client.value);

                Impl {
                    name: StringSpan::new(i.ast_variant().name(), i.identifier().span()),
                    prompt_key: (&props.prompt.key_span).into(),
                    prompt: match props.to_prompt() {
                        PromptAst::String(content, _) => PromptPreview::Completion {
                            completion: apply_replacers(i, content.clone()),
                            test_case: None,
                        },
                        PromptAst::Chat(parts, _) => PromptPreview::Chat {
                            test_case: None,
                            chat: parts
                                .iter()
                                .map(|(ctx, text)| RenderedChatMessage {
                                    role: ctx
                                        .map(|c| c.role.0.as_str())
                                        .unwrap_or("system")
                                        .to_string(),
                                    // message: apply_replacers(i, text.clone()),
                                    parts: vec![ChatMessagePart::Text(apply_replacers(
                                        i,
                                        text.clone(),
                                    ))],
                                })
                                .collect::<Vec<_>>(),
                        },
                    },
                    client: match client {
                        Some(c) => ImplClient {
                            identifier: StringSpan::new(c.name(), c.identifier().span()),
                            provider: c.provider().to_string(),
                            model: c.model().map(|m| m.to_string()),
                        },
                        None => ImplClient {
                            identifier: StringSpan::new(&props.client.value, &props.client.span),
                            provider: "".to_string(),
                            model: None,
                        },
                    },
                    client_list: vec![],
                    input_replacers: vec![],
                    output_replacers: vec![],
                }
            })
            .collect::<Vec<_>>()
    } else {
        let _prompt = func.metadata().prompt.as_ref().unwrap();
        let (_client_span, client_ctx, _client_options) = choose_client(schema, func, None)
            .unwrap_or((
                ImplClient {
                    identifier: StringSpan::new(
                        "{{{{ error resolving client }}}}",
                        func.identifier().span(),
                    ),
                    provider: "".to_string(),
                    model: None,
                },
                RenderContext_Client {
                    name: "{{{{ error rendering ctx.client.name }}}}".to_string(),
                    provider: "{{{{ error rendering ctx.client.provider }}}}".to_string(),
                },
                vec![],
            ));

        let selected_test = (match selected_test_name {
            Some(name) => func.walk_tests().find(|t| t.name() == name),
            None => None,
        })
        .or(func.walk_tests().next());

        let _test_case_name = selected_test.map(|t| t.name().to_string());
        // TODO: find out which properties in the testcase inputs are images, recursively.

        let _args = selected_test
            .map(
                // DO NOT LAND
                |t| match serde_json::from_str(&t.test_case().content.to_string()) {
                    Ok(serde_json::Value::Object(map)) => map,
                    _ => serde_json::Map::new(),
                },
            )
            .unwrap_or(serde_json::Map::new());

        let _render_ctx = RenderContext {
            client: client_ctx,
            output_format: func
                .output_format(&schema.db, func.identifier().span())
                .unwrap_or(format!(
                    "{{{{ Unable to generate ctx.output_format for {} }}}}",
                    func.name()
                )),
            env: HashMap::new(),
        };

        vec![]
        // let rendered = render_prompt(prompt.value(), &args, &render_ctx, template_string_macros);
        // vec![Impl {
        //     name: StringSpan::new("default_config", func.identifier().span()),
        //     prompt_key: prompt.span().into(),
        //     prompt: match rendered {
        //         Ok(internal_baml_jinja::RenderedPrompt::Completion(completion)) => {
        //             PromptPreview::Completion {
        //                 completion,
        //                 test_case: test_case_name,
        //             }
        //         }
        //         Ok(internal_baml_jinja::RenderedPrompt::Chat(chat)) => PromptPreview::Chat {
        //             chat,
        //             test_case: test_case_name,
        //         },
        //         Err(err) => PromptPreview::Error {
        //             error: format!("{err:#}"),
        //             test_case: test_case_name,
        //         },
        //     },
        //     client: client_span,
        //     client_list: client_options,
        //     input_replacers: vec![],
        //     output_replacers: vec![],
        // }]
    }
}

fn print_diagnostics(diagnostics: Vec<MiniError>, response: Option<Value>) -> String {
    json!({
        "ok": response.is_some(),
        "diagnostics": diagnostics,
        "response": response,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    // use expect_test::expect;
    // use indoc::indoc;

    fn lint(s: &str) -> String {
        let result = super::run(s);
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();

        serde_json::to_string_pretty(&value).unwrap()
    }

    // #[test]
    // fn deprecated_preview_features_should_give_a_warning() {
    //     let dml = indoc! {r#"
    //         datasource db {
    //           provider = "postgresql"
    //           url      = env("DATABASE_URL")
    //         }

    //         generator client {
    //           provider = "prisma-client-js"
    //           previewFeatures = ["createMany"]
    //         }

    //         model A {
    //           id  String   @id
    //         }
    //     "#};

    //     let expected = expect![[r#"
    //         [
    //           {
    //             "start": 149,
    //             "end": 163,
    //             "text": "Preview feature \"createMany\" is deprecated. The functionality can be used without specifying it as a preview feature.",
    //             "is_warning": true
    //           }
    //         ]"#]];

    //     expected.assert_eq(&lint(dml));
    // }
}
