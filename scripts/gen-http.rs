#!/usr/bin/env -S cargo -Zscript
---cargo
[package]
edition = "2024"
[dependencies]
serde = "1"
serde_json = "1"
---
// #!/usr/bin/env -S cargo +nightly -Zscript not in use because +nightly errors in nix shell
//! Regenerate `domain.http` from `domain.oas3.json`.
//!
//! Run via `cgs codegen-http` (wired into `codegen`, so `regen` covers it):
//!
//! ```sh
//! cargo run -q -p api --all-features --example gen_http
//! ```
//!
//! Output is deterministic (spec maps iterate in sorted key order): a variable
//! header plus one `###` block per operation with example path values, query
//! strings, and JSON bodies. Example values come straight from the spec
//! (`example` → `default` → first `enum` variant, media-level `example` first
//! for bodies); anything undocumented falls back to neutral type shapes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Default host for the generated file (matches `dx serve`).
const DEFAULT_HOST: &str = "https://localhost:8080";

/// A resolved operation parameter.
struct Param {
    name: String,
    location: String,
    schema: serde_json::Value,
}

/// Follow a local `$ref` like `#/components/schemas/status` inside `doc`.
fn resolve_ref<'a>(doc: &'a serde_json::Value, reference: &str) -> Option<&'a serde_json::Value> {
    let path = reference.strip_prefix("#/")?;
    let mut current = doc;
    for segment in path.split('/') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Resolve either an inline schema or a `$ref` wrapper.
fn resolve_schema<'a>(
    doc: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> Option<&'a serde_json::Value> {
    match schema.get("$ref").and_then(serde_json::Value::as_str) {
        Some(reference) => resolve_ref(doc, reference),
        None => Some(schema),
    }
}

fn str_field(schema: &serde_json::Value, key: &str) -> Option<String> {
    schema
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn int_field(schema: &serde_json::Value, key: &str) -> Option<i64> {
    schema.get(key).and_then(serde_json::Value::as_i64)
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
        Some("integer") => int_field(schema, "minimum")
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::from(1)),
        Some("number") => serde_json::Value::from(1.0),
        Some("boolean") => serde_json::Value::from(true),
        Some("array") => schema
            .get("items")
            .map(|items| serde_json::Value::Array(vec![example_scalar(items)]))
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        _ => serde_json::Value::String(String::new()),
    }
}

/// Render a scalar example as URL text.
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

/// Collect path-level and operation-level parameters, operation entries win
/// on `(in, name)` collisions.
fn collect_params(
    doc: &serde_json::Value,
    item: &serde_json::Value,
    operation: &serde_json::Value,
) -> Vec<Param> {
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
            let Some(name) = str_field(resolved, "name") else {
                continue;
            };
            let Some(location) = str_field(resolved, "in") else {
                continue;
            };
            let schema = resolved
                .get("schema")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            params
                .retain(|existing: &Param| existing.name != name || existing.location != location);
            params.push(Param {
                name,
                location,
                schema,
            });
        }
    }
    params
}

fn is_method(key: &str) -> bool {
    matches!(
        key,
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace"
    )
}

/// Human label for a request block: description, then operation id.
fn label(operation: &serde_json::Value, method: &str, path: &str) -> String {
    str_field(operation, "description")
        .or_else(|| str_field(operation, "operationId"))
        .unwrap_or_else(|| format!("{method} {path}"))
}

/// Pretty-print with the 4-space style of the previous hand-written file.
fn pretty_json(value: &serde_json::Value) -> Result<String, String> {
    use serde::Serialize as _;
    let mut buffer = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(
        &mut buffer,
        serde_json::ser::PrettyFormatter::with_indent(b"    "),
    );
    value
        .serialize(&mut serializer)
        .map_err(|err| format!("cannot encode example body: {err}"))?;
    String::from_utf8(buffer).map_err(|err| format!("example body is not UTF-8: {err}"))
}

/// Render one `###` block for an operation: request line plus an example
/// JSON body when the operation takes one. Operations without a body still
/// render their request line.
fn render_operation(
    doc: &serde_json::Value,
    path: &str,
    method: &str,
    operation: &serde_json::Value,
    params: &[Param],
) -> Result<Vec<String>, String> {
    let mut url = path.to_owned();
    let mut query = Vec::new();
    for param in params {
        let schema = resolve_schema(doc, &param.schema).unwrap_or(&param.schema);
        if param.location == "path" {
            let placeholder = format!("{{{}}}", param.name);
            url = url.replace(&placeholder, &display_scalar(&example_scalar(schema)));
        } else if param.location == "query" {
            let text = display_scalar(&example_scalar(schema));
            query.push(format!("{}={text}", param.name));
        }
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }

    let mut block = vec![
        "###".to_owned(),
        format!("# {}", label(operation, method, path)),
        format!("{} {{{{host}}}}{url}", method.to_uppercase()),
    ];

    let content = operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
        .and_then(serde_json::Value::as_object);
    if let Some(content) = content {
        let media = content
            .keys()
            .find(|media| media.as_str() == "application/json")
            .or_else(|| content.keys().next());
        if let Some(media) = media {
            block.push(format!("Content-Type: {media}"));
            let entry = content.get(media);
            // A media-type-level `example` is used verbatim when present.
            let rendered = match entry.and_then(|entry| entry.get("example")) {
                Some(example) => pretty_json(example)?,
                None => {
                    let schema = entry
                        .and_then(|entry| entry.get("schema"))
                        .and_then(|schema| resolve_schema(doc, schema));
                    match schema {
                        Some(resolved) => match example_object(doc, resolved) {
                            Some(object) => pretty_json(&serde_json::Value::Object(object))?,
                            None => "{}".to_owned(),
                        },
                        None => "{}".to_owned(),
                    }
                }
            };
            block.push(rendered);
        }
    }
    Ok(block)
}

fn generate(spec_path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(spec_path)
        .map_err(|err| format!("cannot read {}: {err}", spec_path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| format!("invalid spec JSON: {err}"))?;

    // `domain.oas3.json` -> `domain`: `file_stem` only strips the last
    // extension, so peel a trailing `.oas3` as well.
    let stem = spec_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("domain");
    let app = stem.strip_suffix(".oas3").unwrap_or(stem).to_owned();
    let revision = doc
        .get("info")
        .and_then(|info| str_field(info, "version"))
        .unwrap_or_else(|| "0.1.0".to_owned());

    let mut out = vec![
        "# Generated from domain.oas3.json — do not edit manually. Regenerate with `cgs codegen`."
            .to_owned(),
        format!("@host={DEFAULT_HOST}"),
        format!("@app={app}"),
        format!("@revision={revision}"),
        "@profile=dev".to_owned(),
    ];

    let paths = doc
        .get("paths")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "spec has no paths object".to_string())?;
    for (path, item) in paths {
        let Some(item_object) = item.as_object() else {
            continue;
        };
        for (method, operation) in item_object {
            if !is_method(method) {
                continue;
            }
            let params = collect_params(&doc, item, operation);
            out.extend(render_operation(&doc, path, method, operation, &params)?);
            out.push(String::new());
        }
    }
    // Drop the trailing blank line, then terminate the file.
    out.pop();
    out.push(String::new());
    Ok(out.join("\n"))
}

fn real_main() -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let spec_path = manifest.join("../domain.oas3.json");
    let http_path = manifest.join("../domain.http");
    let rendered = generate(&spec_path)?;
    std::fs::write(&http_path, rendered)
        .map_err(|err| format!("cannot write {}: {err}", http_path.display()))?;
    println!("wrote {}", http_path.display());
    Ok(())
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
