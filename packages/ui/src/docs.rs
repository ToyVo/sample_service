use dioxus::prelude::*;

use crate::components::{
    badge::{Badge, BadgeVariant},
    button::{Button, ButtonVariant},
    card::{Card, CardContent, CardDescription, CardHeader, CardTitle},
    collapsible::{Collapsible, CollapsibleContent, CollapsibleList, CollapsibleTrigger},
    separator::Separator,
    tooltip::{Tooltip, TooltipContent, TooltipTrigger},
};

// The OpenAPI document, embedded at compile time (same file the axum
// `/api` route and codegen read). The reference below renders from
// this data, so no reflection or runtime fetching is involved.
const SPEC: &str = include_str!("../../../domain.oas3.json");

/// One documented endpoint extracted from the spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
    pub description: String,
    pub params: Vec<ParamInfo>,
    /// Pretty-printed example JSON body, when the operation takes one.
    pub body_example: Option<String>,
}

/// One path or query parameter with a prefilled example value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamInfo {
    pub name: String,
    pub location: String,
    pub example: String,
}

fn is_method(key: &str) -> bool {
    matches!(
        key,
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
    )
}

/// Human label for an operation: description, then operation id.
fn label(operation: &serde_json::Value, method: &str, path: &str) -> String {
    let description = operation
        .get("description")
        .and_then(serde_json::Value::as_str);
    let operation_id = operation
        .get("operationId")
        .and_then(serde_json::Value::as_str);
    description
        .or(operation_id)
        .map_or_else(|| format!("{method} {path}"), str::to_owned)
}

/// Example value straight from the spec: `example` → `default` → first
/// `enum` variant. Anything else falls back to a neutral type-shaped value
/// so output stays well-formed; undocumented strings render empty rather
/// than guessed, which visibly signals missing spec data.
fn example_scalar(schema: &serde_json::Value) -> serde_json::Value {
    if let Some(example) = schema.get("example") {
        return example.clone();
    }
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    if let Some(first) = schema
        .get("enum")
        .and_then(serde_json::Value::as_array)
        .and_then(|variants| variants.first())
    {
        return first.clone();
    }
    match schema.get("type").and_then(serde_json::Value::as_str) {
        Some("integer") => schema
            .get("minimum")
            .and_then(serde_json::Value::as_i64)
            .map_or_else(|| serde_json::Value::from(1), serde_json::Value::from),
        Some("number") => serde_json::Value::from(1.0),
        Some("boolean") => serde_json::Value::from(true),
        Some("array") => schema.get("items").map_or_else(
            || serde_json::Value::Array(Vec::new()),
            |items| serde_json::Value::Array(vec![example_scalar(items)]),
        ),
        _ => serde_json::Value::String(String::new()),
    }
}

/// Render a scalar example as form-field text.
fn display_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Build an example JSON object from an object schema's properties.
fn example_object(
    doc: &serde_json::Value,
    schema: &serde_json::Value,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let properties = schema.get("properties")?.as_object()?;
    let mut object = serde_json::Map::new();
    for (key, property) in properties {
        let resolved = resolve_schema(doc, property)?;
        if resolved.get("type").and_then(serde_json::Value::as_str) == Some("object") {
            object.insert(
                key.clone(),
                serde_json::Value::Object(example_object(doc, resolved)?),
            );
        } else {
            object.insert(key.clone(), example_scalar(resolved));
        }
    }
    Some(object)
}

/// Follow a local `$ref` like `#/components/schemas/status` inside `doc`.
fn resolve_ref<'a>(doc: &'a serde_json::Value, reference: &str) -> Option<&'a serde_json::Value> {
    let mut current = doc;
    for segment in reference.strip_prefix("#/")?.split('/') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Resolve an inline schema or a `$ref` wrapper.
fn resolve_schema<'a>(
    doc: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> Option<&'a serde_json::Value> {
    schema
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .map_or(Some(schema), |reference| resolve_ref(doc, reference))
}

/// Collect path-level and operation-level parameters, with operation entries
/// winning `(in, name)` collisions.
fn collect_params(
    doc: &serde_json::Value,
    item: &serde_json::Value,
    operation: &serde_json::Value,
) -> Vec<ParamInfo> {
    let mut params = Vec::new();
    for source in [item.get("parameters"), operation.get("parameters")] {
        let list = source
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in list {
            let resolved = entry
                .get("$ref")
                .and_then(serde_json::Value::as_str)
                .and_then(|reference| resolve_ref(doc, reference))
                .unwrap_or(&entry);
            let (Some(name), Some(location)) = (
                resolved.get("name").and_then(serde_json::Value::as_str),
                resolved.get("in").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            if location != "path" && location != "query" {
                continue;
            }
            let schema = resolved
                .get("schema")
                .and_then(|schema| resolve_schema(doc, schema))
                .unwrap_or(resolved);
            let example = display_scalar(&example_scalar(schema));
            params.retain(|existing: &ParamInfo| {
                existing.name != name || existing.location != location
            });
            params.push(ParamInfo {
                name: name.to_owned(),
                location: location.to_owned(),
                example,
            });
        }
    }
    params
}

/// Pretty-printed example JSON body for an operation, if it takes a JSON one.
fn example_body(doc: &serde_json::Value, operation: &serde_json::Value) -> Option<String> {
    let content = operation.get("requestBody")?.get("content")?.as_object()?;
    let media = content
        .keys()
        .find(|media| media.as_str() == "application/json")
        .or_else(|| content.keys().next())?;
    let entry = content.get(media)?;
    // A media-type-level `example` is used verbatim when present.
    if let Some(example) = entry.get("example") {
        return serde_json::to_string_pretty(example).ok();
    }
    let schema = resolve_schema(doc, entry.get("schema")?)?;
    let object = example_object(doc, schema)?;
    serde_json::to_string_pretty(&serde_json::Value::Object(object)).ok()
}

/// Extract every endpoint from the spec. Iteration follows the document's
/// sorted key order (`serde_json` maps are sorted), so output is stable.
///
/// # Errors
///
/// Returns a message when `spec` is not valid JSON or has no `paths` object.
pub fn parse_endpoints(spec: &str) -> Result<Vec<Endpoint>, String> {
    let doc: serde_json::Value =
        serde_json::from_str(spec).map_err(|err| format!("invalid spec: {err}"))?;
    let paths = doc
        .get("paths")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "spec has no paths object".to_string())?;
    let mut endpoints = Vec::new();
    // Sort explicitly: `serde_json` map order depends on the `preserve_order`
    // feature (unified on or off by the dependency graph), so document order
    // is not guaranteed. Sorted output also matches `domain.http`.
    let mut ordered: Vec<(&String, &serde_json::Value)> = paths.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    for (path, item) in ordered {
        let Some(operations) = item.as_object() else {
            continue;
        };
        let mut methods: Vec<(&String, &serde_json::Value)> = operations.iter().collect();
        methods.sort_by(|a, b| a.0.cmp(b.0));
        for (method, operation) in methods {
            if is_method(method) {
                endpoints.push(Endpoint {
                    method: method.to_uppercase(),
                    path: path.clone(),
                    description: label(operation, method, path),
                    params: collect_params(&doc, item, operation),
                    body_example: example_body(&doc, operation),
                });
            }
        }
    }
    Ok(endpoints)
}

fn method_variant(method: &str) -> BadgeVariant {
    match method {
        "GET" => BadgeVariant::Secondary,
        "POST" => BadgeVariant::Primary,
        "DELETE" => BadgeVariant::Destructive,
        _ => BadgeVariant::Outline,
    }
}

/// API reference rendered from the embedded `OpenAPI` document, with links into
/// the interactive console.
///
/// `console_target` is supplied by the host app (`web` / `desktop` / `mobile`)
/// as `Route::Console {}.into()`, so this shared crate never names a concrete
/// `Routable` enum yet `Link` still does client-side navigation.
///
/// Built from `dioxus-components` Card, Collapsible, Badge, Separator, Button,
/// and Tooltip. Each endpoint is an independent `Collapsible` (own open
/// `Memo`, instant unmount on close) instead of `Accordion` (shared
/// `Signal<Vec<usize>>` fan-out + `height`/`grid-template-rows` main-thread
/// animation). Content animation is `opacity`/`transform` only.
#[component]
pub fn Docs(console_target: NavigationTarget) -> Element {
    let endpoints = use_memo(|| parse_endpoints(SPEC));
    let navigator = use_navigator();
    match endpoints() {
        Ok(endpoints) => rsx! {
            Card {
                style: "max-width: 800px; margin: 24px auto;",
                CardHeader {
                    CardTitle { "API reference" }
                    CardDescription {
                        "Rendered from the embedded OpenAPI document. "
                        a { href: "/api", "Raw spec" }
                        " · "
                        Link {
                            to: console_target,
                            "Try it in the console"
                        }
                    }
                }
                CardContent {
                    Separator { style: "margin: 12px 0;" }
                    CollapsibleList {
                        for endpoint in endpoints.iter() {
                            Collapsible {
                                key: "{endpoint.method} {endpoint.path}",
                                CollapsibleTrigger {
                                    Badge {
                                        variant: method_variant(&endpoint.method),
                                        "{endpoint.method}"
                                    }
                                    code { style: "margin-left: 8px;", "{endpoint.path}" }
                                }
                                CollapsibleContent {
                                    p { "{endpoint.description}" }
                                    if !endpoint.params.is_empty() {
                                        p { style: "opacity: 0.7;",
                                            "{endpoint.params.len()} param(s): "
                                            for param in &endpoint.params {
                                                Badge {
                                                    key: "{param.location}:{param.name}",
                                                    variant: BadgeVariant::Outline,
                                                    "{param.location} {param.name}"
                                                }
                                                " "
                                            }
                                        }
                                    }
                                    if let Some(body) = &endpoint.body_example {
                                        pre {
                                            style: "overflow: auto; opacity: 0.8;",
                                            {body.clone()}
                                        }
                                    }
                                    Tooltip {
                                        TooltipTrigger {
                                            Button {
                                                variant: ButtonVariant::Link,
                                                onclick: {
                                                    let target = format!(
                                                        "/console#{}:{}",
                                                        endpoint.method,
                                                        endpoint.path
                                                    );
                                                    move |_| {
                                                        navigator.push(target.clone());
                                                    }
                                                },
                                                "Try it →"
                                            }
                                        }
                                        TooltipContent { "Open in the API console" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        Err(message) => rsx! {
            Card {
                style: "max-width: 800px; margin: 24px auto;",
                CardHeader {
                    CardTitle { "API reference" }
                    CardDescription { "Could not parse the embedded spec: {message}" }
                }
            }
        },
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_operation() {
        let endpoints = parse_endpoints(SPEC).expect("spec parses");
        let rendered: Vec<String> = endpoints
            .iter()
            .map(|endpoint| endpoint.method.clone() + " " + &endpoint.path)
            .collect();
        assert_eq!(
            rendered,
            vec![
                "GET /api",
                "GET /api/samples",
                "POST /api/samples",
                "DELETE /api/samples/{sampleId}",
                "GET /api/samples/{sampleId}",
                "PATCH /api/samples/{sampleId}",
            ]
        );
    }

    #[test]
    fn labels_prefer_descriptions() {
        let endpoints = parse_endpoints(SPEC).expect("spec parses");
        let post = endpoints
            .iter()
            .find(|endpoint| endpoint.method == "POST")
            .expect("post exists");
        assert_eq!(post.description, "Creates a sample");
    }

    #[test]
    fn invalid_spec_reports_an_error() {
        assert!(parse_endpoints("{oops").is_err());
        assert!(parse_endpoints("{}").is_err());
    }

    #[test]
    fn list_operation_has_pagination_params_and_no_body() {
        let endpoints = parse_endpoints(SPEC).expect("spec parses");
        let list = endpoints
            .iter()
            .find(|endpoint| endpoint.method == "GET" && endpoint.path == "/api/samples")
            .expect("list exists");
        let params: Vec<(&str, &str)> = list
            .params
            .iter()
            .map(|param| (param.location.as_str(), param.example.as_str()))
            .collect();
        assert!(params.contains(&("query", "0")), "offset default");
        assert!(params.contains(&("query", "100")), "limit default");
        assert_eq!(list.body_example, None);
    }

    #[test]
    fn item_operations_share_the_path_param() {
        let endpoints = parse_endpoints(SPEC).expect("spec parses");
        for endpoint in endpoints
            .iter()
            .filter(|endpoint| endpoint.path.contains("{sampleId}"))
        {
            let id = endpoint
                .params
                .iter()
                .find(|param| param.name == "sampleId")
                .expect("sampleId documented");
            assert_eq!(id.location, "path");
            // Prefill is never blank: an explicit spec example wins, an
            // absent/empty one falls back to the id heuristic (covered
            // directly in `example_scalars_follow_documented_rules`).
            assert_ne!(id.example, String::new());
        }
    }

    #[test]
    fn create_body_example_has_wire_names_and_status() {
        let endpoints = parse_endpoints(SPEC).expect("spec parses");
        let create = endpoints
            .iter()
            .find(|endpoint| endpoint.method == "POST")
            .expect("create exists");
        let body: serde_json::Value =
            serde_json::from_str(create.body_example.as_ref().expect("example body"))
                .expect("valid json");
        assert_eq!(
            body.get("name").and_then(|name| name.as_str()),
            Some("sample")
        );
        assert_eq!(
            body.get("status").and_then(|status| status.as_str()),
            Some("ACTIVE")
        );
        assert!(
            body.get("imageUri").is_some(),
            "uses wire names, not rust names"
        );
    }

    #[test]
    fn example_scalars_follow_documented_rules() {
        // Explicit spec values always win, in `example` → `default` order.
        assert_eq!(
            example_scalar(&serde_json::json!({"example": "x", "default": "y"})),
            serde_json::json!("x")
        );
        assert_eq!(
            example_scalar(&serde_json::json!({"default": 7})),
            serde_json::json!(7)
        );
        assert_eq!(
            example_scalar(&serde_json::json!({"enum": ["b", "a"]})),
            serde_json::json!("b")
        );
        // Structural fallbacks carry no invented content.
        assert_eq!(
            example_scalar(&serde_json::json!({"type": "integer", "minimum": 5})),
            serde_json::json!(5)
        );
        assert_eq!(
            example_scalar(&serde_json::json!({"type": "integer"})),
            serde_json::json!(1)
        );
        assert_eq!(
            example_scalar(&serde_json::json!({"type": "string"})),
            serde_json::json!("")
        );
        assert_eq!(
            example_scalar(&serde_json::json!({
                "type": "array",
                "items": {"enum": ["b", "a"]}
            })),
            serde_json::json!(["b"])
        );
    }
}
