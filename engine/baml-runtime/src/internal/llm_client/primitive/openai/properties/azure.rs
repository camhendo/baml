use std::collections::HashMap;

use anyhow::{Context, Result};
use internal_baml_core::ir::ClientWalker;

use crate::{internal::llm_client::AllowedMetadata, RuntimeContext};

use super::PostRequestProperities;

pub fn resolve_properties(
    mut properties: HashMap<String, serde_json::Value>,
    ctx: &RuntimeContext,
) -> Result<PostRequestProperities> {
    // POST https://{your-resource-name}.openai.azure.com/openai/deployments/{deployment-id}/chat/completions?api-version={api-version}

    let default_role = properties
        .remove("default_role")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "system".to_string());
    let allowed_metadata = match properties.remove("allowed_role_metadata") {
        Some(allowed_metadata) => serde_json::from_value(allowed_metadata)
            .context("allowed_role_metadata must be 'all', 'none', or ['key1', 'key2']")?,
        None => AllowedMetadata::None,
    };
    // Ensure that either (resource_name, deployment_id) or base_url is provided
    let base_url = properties.remove("base_url");
    let resource_name = properties.remove("resource_name");
    let deployment_id = properties.remove("deployment_id");
    let api_version = properties.remove("api_version");

    let base_url = match (base_url, resource_name, deployment_id) {
        (Some(base_url), None, None) => base_url
            .as_str()
            .map(|s| s.to_string())
            .context("base_url must be a string")?,
        (None, Some(resource_name), Some(deployment_id)) => {
            format!(
                "https://{}.openai.azure.com/openai/deployments/{}",
                resource_name
                    .as_str()
                    .context("resource_name must be a string")?,
                deployment_id
                    .as_str()
                    .context("deployment_id must be a string")?
            )
        }
        _ => anyhow::bail!("Either base_url or (resource_name, deployment_id) must be provided"),
    };

    let api_key = properties
        .remove("api_key")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .or_else(|| ctx.env.get("AZURE_OPENAI_API_KEY").map(|s| s.to_string()));

    let headers = properties.remove("headers").map(|v| {
        if let Some(v) = v.as_object() {
            v.iter()
                .map(|(k, v)| {
                    Ok((
                        k.to_string(),
                        match v {
                            serde_json::Value::String(s) => s.to_string(),
                            _ => anyhow::bail!("Header '{k}' must be a string"),
                        },
                    ))
                })
                .collect::<Result<HashMap<String, String>>>()
        } else {
            Ok(Default::default())
        }
    });
    let mut headers = match headers {
        Some(h) => h?,
        None => Default::default(),
    };

    if let Some(api_key) = &api_key {
        headers.insert("API-KEY".to_string(), api_key.clone());
    }

    let mut query_params = HashMap::new();
    if let Some(v) = api_version {
        if let Some(v) = v.as_str() {
            query_params.insert("api-version".to_string(), v.to_string());
        } else {
            anyhow::bail!("api_version must be a string")
        }
    };

    properties
        .entry("max_tokens".into())
        .or_insert_with(|| 4096.into());

    Ok(PostRequestProperities {
        default_role,
        base_url,
        api_key: None,
        headers,
        properties,
        allowed_metadata,
        // Replace proxy_url with code below to disable proxying
        // proxy_url: None,
        proxy_url: ctx.env.get("BOUNDARY_PROXY_URL").map(|s| s.to_string()),
        query_params,
    })
}
