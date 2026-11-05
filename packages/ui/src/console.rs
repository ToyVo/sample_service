use std::collections::HashMap;

use dioxus::prelude::*;

use super::docs::{Endpoint, ParamInfo, parse_endpoints};
use crate::components::{
    badge::{Badge, BadgeVariant},
    button::Button,
    card::{Card, CardContent, CardDescription, CardHeader, CardTitle},
    input::Input,
    label::Label,
    progress::Progress,
    select::{Select, SelectGroup, SelectGroupLabel, SelectOption},
    separator::Separator,
    skeleton::Skeleton,
    switch::Switch,
    textarea::Textarea,
    tooltip::{Tooltip, TooltipContent, TooltipTrigger},
};

const SPEC: &str = include_str!("../../../domain.oas3.json");

/// Outcome of one console request.
#[derive(Debug, Clone, Default)]
struct ConsoleResponse {
    status: Option<u16>,
    body: String,
    error: Option<String>,
}

/// Fill `{placeholders}` from path values and append non-empty query values.
fn build_url(
    template: &str,
    path_values: &HashMap<String, String>,
    query_values: &HashMap<String, String>,
) -> String {
    let mut url = template.to_owned();
    // Deterministic order keeps tests stable regardless of hasher state.
    let mut names: Vec<&String> = path_values.keys().collect();
    names.sort();
    for name in names {
        let placeholder = "{".to_owned() + name + "}";
        if let Some(value) = path_values.get(name) {
            url = url.replace(&placeholder, value);
        }
    }
    let mut pairs: Vec<(&String, &String)> = query_values
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .collect();
    pairs.sort();
    if pairs.is_empty() {
        return url;
    }
    url.push('?');
    let mut first = true;
    for (name, value) in pairs {
        if !first {
            url.push('&');
        }
        first = false;
        url.push_str(name);
        url.push('=');
        url.push_str(value);
    }
    url
}

/// Prefix a same-origin path with the page origin.
///
/// `reqwest` validates URLs in Rust before handing them to the browser
/// `fetch` API, so a relative path fails with a builder error and no network
/// request is ever sent. Event handlers only execute client-side, but this
/// code must still compile for the server target, hence the split.
pub fn absolute_url(path: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default();
    #[cfg(not(target_arch = "wasm32"))]
    let origin = String::new();
    format!("{origin}{path}")
}

/// The console issues same-origin requests, so relative URLs resolve against
/// whatever host serves the app. `reqwest` runs on the browser `fetch` API
/// client-side; server-side rendering never executes these handlers.
pub async fn try_request(
    method: String,
    url: String,
    body: Option<String>,
) -> Result<(u16, String), String> {
    let client = reqwest::Client::new();
    let mut request = match method.as_str() {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        _ => client.get(url),
    };
    if let Some(text) = body {
        request = request
            .header("Content-Type", "application/json")
            .body(text);
    }
    let response = request.send().await.map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    let text = response.text().await.map_err(|err| err.to_string())?;
    Ok((status, text))
}

/// Pretty-print a JSON body for display, falling back to raw text.
fn display_body(text: &str, pretty: bool) -> String {
    if !pretty {
        return text.to_owned();
    }
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_owned())
}

fn endpoint_key(endpoint: &Endpoint) -> String {
    format!("{} {}", endpoint.method, endpoint.path)
}

fn method_variant(method: &str) -> BadgeVariant {
    match method {
        "GET" => BadgeVariant::Secondary,
        "POST" => BadgeVariant::Primary,
        "DELETE" => BadgeVariant::Destructive,
        _ => BadgeVariant::Outline,
    }
}

/// Interactive API console rendered from the embedded `OpenAPI` document.
///
/// `docs_target` is supplied by the host app (`web` / `desktop` / `mobile`)
/// as `Route::Docs {}.into()`, so this shared crate never names a concrete
/// `Routable` enum yet `Link` still does client-side navigation.
///
/// Built from `dioxus-components` Card, Select, Label, Input, Textarea,
/// Button, Badge, Separator, Progress, Switch, and Tooltip. Endpoint
/// metadata comes from [`parse_endpoints`], so no reflection or runtime
/// fetching is involved; requests go to the same origin.
#[component]
pub fn Console(docs_target: NavigationTarget) -> Element {
    let endpoints = use_memo(|| parse_endpoints(SPEC).unwrap_or_default());
    let (init_paths, init_queries, init_body) = endpoints()
        .first()
        .map(endpoint_defaults)
        .unwrap_or_default();
    let mut selected = use_signal(|| 0_usize);
    let mut path_values = use_signal(move || init_paths);
    let mut query_values = use_signal(move || init_queries);
    let mut body_text = use_signal(move || init_body);
    let mut pending = use_signal(|| false);
    let mut pretty = use_signal(|| true);
    let mut response = use_signal(ConsoleResponse::default);

    let current: Option<Endpoint> = endpoints().get(selected()).cloned();
    let current_key = current.as_ref().map(endpoint_key).unwrap_or_default();

    rsx! {
        Card {
            style: "max-width: 800px; margin: 24px auto;",
            CardHeader {
                CardTitle { "API console" }
                CardDescription {
                    "Try the endpoints live against this server. Reference: "
                    Link {
                        to: docs_target,
                        "API Docs"
                    }
                    " · Spec: "
                    a { href: "/api", "api" }
                }
            }
            CardContent {
                Label { html_for: "endpoint-select", "Endpoint" }
                if endpoints().is_empty() {
                    p { "No endpoints found in the embedded spec." }
                } else {
                    Select::<String> {
                        id: "endpoint-select",
                        default_value: current_key,
                        on_value_change: move |value: Option<String>| {
                            if let Some(key) = value
                                && let Some(index) = endpoints()
                                    .iter()
                                    .position(|endpoint| endpoint_key(endpoint) == key)
                                && let Some(endpoint) = endpoints().get(index).cloned()
                            {
                                let (paths, queries, body) = endpoint_defaults(&endpoint);
                                path_values.set(paths);
                                query_values.set(queries);
                                body_text.set(body);
                                response.set(ConsoleResponse::default());
                                selected.set(index);
                            }
                        },
                        SelectGroup {
                            SelectGroupLabel { "Endpoints" }
                            for (index, endpoint) in endpoints().iter().enumerate() {
                                SelectOption::<String> {
                                    key: "{endpoint.method} {endpoint.path}",
                                    index,
                                    value: endpoint_key(endpoint),
                                    text_value: endpoint_key(endpoint),
                                    "{endpoint.method} {endpoint.path}"
                                }
                            }
                        }
                    }
                }
                Separator { style: "margin: 12px 0;" }
                if let Some(endpoint) = current {
                    div {
                        style: "display: flex; gap: 8px; align-items: center; margin-bottom: 8px;",
                        Badge {
                            variant: method_variant(&endpoint.method),
                            "{endpoint.method}"
                        }
                        code { "{endpoint.path}" }
                    }
                    p { style: "opacity: 0.7;", "{endpoint.description}" }
                    for param in &endpoint.params {
                        div {
                            key: "{param.location}:{param.name}",
                            style: "margin: 8px 0;",
                            Label {
                                html_for: "param-{param.location}-{param.name}",
                                "{param.location} {param.name}"
                            }
                            Input {
                                id: "param-{param.location}-{param.name}",
                                value: param_values(&path_values(), &query_values(), param),
                                oninput: {
                                    let name = param.name.clone();
                                    let location = param.location.clone();
                                    move |event: FormEvent| {
                                        let target = if location == "path" {
                                            &mut path_values
                                        } else {
                                            &mut query_values
                                        };
                                        target.write().insert(name.clone(), event.value());
                                    }
                                },
                            }
                        }
                    }
                    if endpoint.body_example.is_some() {
                        div { style: "margin: 8px 0;",
                            Label { html_for: "console-body", "JSON body" }
                            Textarea {
                                id: "console-body",
                                value: body_text(),
                                rows: "8",
                                style: "width: 100%; font-family: monospace;",
                                oninput: move |event: FormEvent| body_text.set(event.value()),
                            }
                        }
                    }
                    div {
                        style: "display: flex; gap: 12px; align-items: center; margin: 12px 0;",
                        Tooltip {
                            TooltipTrigger {
                                Button {
                                    disabled: pending(),
                                    onclick: move |_| {
                                            if pending() {
                                                return;
                                            }
                                            pending.set(true);
                                            let url = absolute_url(
                                                &build_url(
                                                    &endpoint.path,
                                                    &path_values(),
                                                    &query_values(),
                                                ),
                                            );
                                            let body = (!body_text().is_empty()).then(|| {
                                                match serde_json::from_str::<
                                                    serde_json::Value,
                                                >(&body_text())
                                                {
                                                    Ok(_) => Ok(body_text()),
                                                    Err(err) => {
                                                        Err(format!("invalid JSON body: {err}"))
                                                    }
                                                }
                                            });
                                            let method = endpoint.method.clone();
                                            let pretty_print = pretty();
                                            spawn(async move {
                                                let outcome = match body {
                                                    Some(Err(message)) => ConsoleResponse {
                                                        error: Some(message),
                                                        ..Default::default()
                                                    },
                                                    Some(Ok(text)) => {
                                                        match try_request(method, url, Some(text))
                                                            .await
                                                        {
                                                            Ok((status, raw)) => ConsoleResponse {
                                                                status: Some(status),
                                                                body: display_body(
                                                                    &raw,
                                                                    pretty_print,
                                                                ),
                                                                error: None,
                                                            },
                                                            Err(message) => ConsoleResponse {
                                                                error: Some(message),
                                                                ..Default::default()
                                                            },
                                                        }
                                                    }
                                                    None => {
                                                        match try_request(method, url, None).await {
                                                            Ok((status, raw)) => ConsoleResponse {
                                                                status: Some(status),
                                                                body: display_body(
                                                                    &raw,
                                                                    pretty_print,
                                                                ),
                                                                error: None,
                                                            },
                                                            Err(message) => ConsoleResponse {
                                                                error: Some(message),
                                                                ..Default::default()
                                                            },
                                                        }
                                                    }
                                                };
                                                response.set(outcome);
                                                pending.set(false);
                                            });
                                        },
                                    if pending() { "Sending…" } else { "Send" }
                                }
                            }
                            TooltipContent { "Send request to this server" }
                        }
                        Label { html_for: "pretty-toggle", "Pretty-print" }
                        Switch {
                            id: "pretty-toggle",
                            checked: pretty(),
                            on_checked_change: move |value| pretty.set(value),
                        }
                    }
                    if pending() {
                        Progress { value: 50.0 }
                        Skeleton { style: "height: 80px; margin-top: 8px;" }
                    }
                    {
                        let outcome = response();
                        rsx! {
                            Separator { style: "margin: 12px 0;" }
                            h3 { "Response" }
                            if let Some(status) = outcome.status {
                                Badge {
                                    variant: if status < 400 { BadgeVariant::Secondary } else { BadgeVariant::Destructive },
                                    "Status: {status}"
                                }
                            }
                            if let Some(message) = outcome.error {
                                p { style: "color: var(--primary-error-color);",
                                    "Error: {message}"
                                }
                            }
                            pre {
                                style: "overflow: auto; background: rgba(0,0,0,0.3); padding: 8px; border-radius: 4px;",
                                "{outcome.body}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Default form state for an endpoint: path values, query values, body text.
fn endpoint_defaults(
    endpoint: &Endpoint,
) -> (HashMap<String, String>, HashMap<String, String>, String) {
    let mut paths = HashMap::new();
    let mut queries = HashMap::new();
    for param in &endpoint.params {
        let values = if param.location == "path" {
            &mut paths
        } else {
            &mut queries
        };
        values.insert(param.name.clone(), param.example.clone());
    }
    let body = endpoint.body_example.clone().unwrap_or_default();
    (paths, queries, body)
}

/// Current value for one parameter input.
fn param_values(
    paths: &HashMap<String, String>,
    queries: &HashMap<String, String>,
    param: &ParamInfo,
) -> String {
    if param.location == "path" {
        paths.get(&param.name).cloned().unwrap_or_default()
    } else {
        queries.get(&param.name).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn id_only() -> (HashMap<String, String>, HashMap<String, String>) {
        let mut paths = HashMap::new();
        paths.insert("sampleId".to_owned(), "12345".to_owned());
        (paths, HashMap::new())
    }

    #[test]
    fn absolute_url_leaves_paths_untouched_off_wasm() {
        // On non-wasm targets there is no `window.location`; the real origin
        // is prepended by the wasm branch in the browser.
        assert_eq!(absolute_url("/samples?limit=1"), "/samples?limit=1");
    }

    #[test]
    fn substitutes_path_params() {
        let (paths, queries) = id_only();
        assert_eq!(
            build_url("/samples/{sampleId}", &paths, &queries),
            "/samples/12345"
        );
    }

    #[test]
    fn appends_non_empty_query_params_sorted() {
        let queries = HashMap::from([
            ("limit".to_owned(), "100".to_owned()),
            ("offset".to_owned(), "0".to_owned()),
            ("empty".to_owned(), String::new()),
        ]);
        assert_eq!(
            build_url("/samples", &HashMap::new(), &queries),
            "/samples?limit=100&offset=0"
        );
    }

    #[test]
    fn leaves_unknown_placeholders_alone() {
        assert_eq!(
            build_url("/samples/{missing}", &HashMap::new(), &HashMap::new()),
            "/samples/{missing}"
        );
    }

    #[test]
    fn defaults_prefill_from_endpoint_examples() {
        let no_params = Endpoint {
            method: "GET".to_owned(),
            path: "/samples".to_owned(),
            description: String::new(),
            params: Vec::new(),
            body_example: None,
        };
        assert_eq!(
            endpoint_defaults(&no_params),
            (HashMap::new(), HashMap::new(), String::new(),)
        );
    }

    #[test]
    fn pretty_prints_json_bodies() {
        assert_eq!(display_body("{\"a\":1}", true), "{\n  \"a\": 1\n}");
        assert_eq!(display_body("not json", true), "not json");
        assert_eq!(display_body("{\"a\":1}", false), "{\"a\":1}");
    }
}
