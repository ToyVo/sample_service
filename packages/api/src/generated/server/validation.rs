//! Offline request-schema validators and normalized public rejections.
#![allow(dead_code)]
use super::errors::{InvalidParameter, ProblemDetails, RequestValidationRejection};
const VALIDATION_SCHEMA: &str = "{\"$schema\":\"http://json-schema.org/draft-04/schema#\",\"definitions\":{\"components\":{\"component_newSample\":{\"description\":\"A sample entity representing a generic resource\",\"properties\":{\"description\":{\"description\":\"A longer description of the sample\",\"maxLength\":2000,\"type\":\"string\"},\"imageUri\":{\"description\":\"Absolute URI to an image representing the sample\",\"maxLength\":500,\"type\":\"string\"},\"name\":{\"description\":\"Descriptive name for the sample\",\"maxLength\":100,\"type\":\"string\"},\"status\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/definitions/components/component_status\"},\"tags\":{\"description\":\"Tags associated with the sample\",\"items\":{\"maxLength\":100,\"type\":\"string\"},\"maxItems\":25,\"type\":\"array\"}},\"required\":[\"name\",\"status\"],\"type\":\"object\"},\"component_sampleUpdate\":{\"description\":\"A sample entity representing a generic resource\",\"properties\":{\"description\":{\"description\":\"A longer description of the sample\",\"maxLength\":2000,\"type\":\"string\"},\"imageUri\":{\"description\":\"Absolute URI to an image representing the sample\",\"maxLength\":500,\"type\":\"string\"},\"name\":{\"description\":\"Descriptive name for the sample\",\"maxLength\":100,\"type\":\"string\"},\"status\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/definitions/components/component_status\"},\"tags\":{\"description\":\"Tags associated with the sample\",\"items\":{\"maxLength\":100,\"type\":\"string\"},\"maxItems\":25,\"type\":\"array\"}},\"type\":\"object\"},\"component_status\":{\"description\":\"Current state of a sample\",\"enum\":[\"ACTIVE\",\"INACTIVE\",\"ARCHIVED\"],\"type\":\"string\"}},\"targets\":{\"v0\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/definitions/components/component_newSample\"},\"v1\":{\"maxLength\":36,\"minLength\":5,\"type\":\"string\"},\"v2\":{\"default\":0,\"minimum\":0,\"type\":\"integer\"},\"v3\":{\"default\":100,\"maximum\":1000,\"minimum\":1,\"type\":\"integer\"},\"v4\":{\"maxLength\":36,\"minLength\":5,\"type\":\"string\"},\"v5\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/definitions/components/component_sampleUpdate\"},\"v6\":{\"maxLength\":36,\"minLength\":5,\"type\":\"string\"}}},\"id\":\"urn:openapi-to-rust:request-validation\"}";
const MAX_VALIDATION_ERRORS: usize = 16usize;
pub(crate) const VALIDATION_TARGET_0_BODY: &str = "#/definitions/targets/v0";
pub(crate) const VALIDATION_TARGET_1_PATH_0: &str = "#/definitions/targets/v1";
pub(crate) const VALIDATION_TARGET_2_QUERY_0: &str = "#/definitions/targets/v2";
pub(crate) const VALIDATION_TARGET_3_QUERY_1: &str = "#/definitions/targets/v3";
pub(crate) const VALIDATION_TARGET_4_PATH_0: &str = "#/definitions/targets/v4";
pub(crate) const VALIDATION_TARGET_5_BODY: &str = "#/definitions/targets/v5";
pub(crate) const VALIDATION_TARGET_6_PATH_0: &str = "#/definitions/targets/v6";
static VALIDATORS: ::std::sync::LazyLock<::std::result::Result<::jsonschema::ValidatorMap, ()>> =
    ::std::sync::LazyLock::new(|| {
        let schema: ::serde_json::Value =
            ::serde_json::from_str(VALIDATION_SCHEMA).map_err(|_| ())?;
        ::jsonschema::options()
            .with_draft(::jsonschema::Draft::Draft4)
            .should_validate_formats(true)
            .with_pattern_options(::jsonschema::PatternOptions::regex())
            .build_map(&schema)
            .map_err(|_| ())
    });
pub(crate) async fn decode_json_body<T>(
    request: ::axum::extract::Request,
    target: &str,
    expected_media_type: &str,
    required: bool,
    max_body_bytes: usize,
) -> ::std::result::Result<::std::option::Option<T>, RequestValidationRejection>
where
    T: ::serde::de::DeserializeOwned,
{
    let (parts, body) = request.into_parts();
    let content_type = parts
        .headers
        .get(::axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let is_json = content_type.is_some_and(|value| media_type_is(value, expected_media_type));
    if !is_json {
        if content_type.is_none() && !required {
            let bytes = read_body(body, max_body_bytes).await?;
            if bytes.is_empty() {
                return Ok(None);
            }
        }
        return Err(unsupported_media_type());
    }
    let bytes = read_body(body, max_body_bytes).await?;
    if bytes.is_empty() {
        return if required {
            Err(malformed_request())
        } else {
            Ok(None)
        };
    }
    let instance: ::serde_json::Value =
        ::serde_json::from_slice(&bytes).map_err(|_| malformed_request())?;
    validate(target, "/body", &instance)?;
    let typed = ::serde_json::from_value(instance).map_err(|_| generated_contract_error())?;
    Ok(Some(typed))
}
pub(crate) fn decode_parameter<T>(
    raw: &str,
    target: &str,
    location: &str,
    string_wire: bool,
) -> ::std::result::Result<T, RequestValidationRejection>
where
    T: ::serde::de::DeserializeOwned + ::serde::Serialize,
{
    let typed = if string_wire {
        let instance = ::serde_json::Value::String(raw.to_string());
        validate(target, location, &instance)?;
        ::serde_json::from_value(instance).map_err(|_| generated_contract_error())?
    } else {
        ::serde_json::from_value(::serde_json::Value::String(raw.to_string()))
            .or_else(|_| ::serde_json::from_str(raw))
            .map_err(|_| malformed_parameter(location))?
    };
    validate_parameter(target, location, &typed)?;
    Ok(typed)
}
pub(crate) fn validate_string_parameter(
    target: &str,
    location: &str,
    raw: &str,
) -> ::std::result::Result<(), RequestValidationRejection> {
    validate(
        target,
        location,
        &::serde_json::Value::String(raw.to_string()),
    )
}
pub(crate) fn validate_parameter<T>(
    target: &str,
    location: &str,
    value: &T,
) -> ::std::result::Result<(), RequestValidationRejection>
where
    T: ::serde::Serialize + ?Sized,
{
    let instance = ::serde_json::to_value(value).map_err(|_| generated_contract_error())?;
    validate(target, location, &instance)
}
pub(crate) fn parse_cookies(
    headers: &::axum::http::HeaderMap,
) -> ::std::result::Result<::std::collections::BTreeMap<String, String>, RequestValidationRejection>
{
    let mut cookies = ::std::collections::BTreeMap::new();
    for header in headers.get_all(::axum::http::header::COOKIE).iter() {
        let line = header
            .to_str()
            .map_err(|_| malformed_parameter("/cookie"))?;
        for field in line.split(';') {
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            let (name, value) = field
                .split_once('=')
                .ok_or_else(|| malformed_parameter("/cookie"))?;
            let name = name.trim();
            if name.is_empty()
                || cookies
                    .insert(name.to_string(), value.to_string())
                    .is_some()
            {
                return Err(malformed_parameter("/cookie"));
            }
        }
    }
    Ok(cookies)
}
async fn read_body(
    body: ::axum::body::Body,
    max_body_bytes: usize,
) -> ::std::result::Result<::axum::body::Bytes, RequestValidationRejection> {
    ::axum::body::to_bytes(body, max_body_bytes)
        .await
        .map_err(|error| {
            let source = ::std::error::Error::source(&error);
            if source.is_some_and(|source| source.is::<::http_body_util::LengthLimitError>()) {
                request_body_too_large()
            } else {
                malformed_request()
            }
        })
}
fn media_type_is(content_type: &str, expected: &str) -> bool {
    let Ok(content_type) = content_type.parse::<::mime::Mime>() else {
        return false;
    };
    let Ok(expected) = expected.parse::<::mime::Mime>() else {
        return false;
    };
    content_type.type_() == expected.type_()
        && content_type.subtype() == expected.subtype()
        && content_type.suffix() == expected.suffix()
        && expected.params().all(|(name, value)| {
            content_type
                .get_param(name)
                .is_some_and(|actual| actual == value)
        })
}
pub(crate) fn validate(
    target: &str,
    location: &str,
    instance: &::serde_json::Value,
) -> ::std::result::Result<(), RequestValidationRejection> {
    let validators = VALIDATORS
        .as_ref()
        .map_err(|_| generated_contract_error())?;
    let Some(validator) = validators.get(target) else {
        return Err(generated_contract_error());
    };
    let mut errors: ::std::vec::Vec<InvalidParameter> = ::std::vec::Vec::new();
    for error in validator.iter_errors(instance) {
        let keyword = error.kind().keyword();
        let (code, message) = public_violation(keyword);
        let mut pointer = format!("{location}{}", error.instance_path());
        if let ::jsonschema::error::ValidationErrorKind::Required { property } = error.kind() {
            if let Some(property) = property.as_str() {
                pointer.push('/');
                pointer.push_str(&escape_pointer_token(property));
            }
        }
        let violation = InvalidParameter {
            code: code.to_string(),
            location: pointer,
            message: message.to_string(),
        };
        if !errors.contains(&violation) {
            errors.push(violation);
            if errors.len() == MAX_VALIDATION_ERRORS {
                break;
            }
        }
    }
    errors.sort_by(|left, right| {
        (&left.location, &left.code, &left.message).cmp(&(
            &right.location,
            &right.code,
            &right.message,
        ))
    });
    if errors.is_empty() {
        return Ok(());
    }
    Err(RequestValidationRejection(ProblemDetails {
        r#type: "https://openapi-to-rust.dev/problems/validation".to_string(),
        title: "Request validation failed".to_string(),
        status: 422,
        code: "request_validation_failed".to_string(),
        errors,
    }))
}
pub(crate) fn malformed_request() -> RequestValidationRejection {
    public_problem(
        400,
        "https://openapi-to-rust.dev/problems/malformed-request",
        "Malformed request",
        "malformed_request",
    )
}
pub(crate) fn malformed_parameter(location: &str) -> RequestValidationRejection {
    parameter_problem(
        400,
        "https://openapi-to-rust.dev/problems/malformed-parameter",
        "Malformed request parameter",
        "malformed_parameter",
        "malformed",
        location,
        "is malformed",
    )
}
pub(crate) fn missing_parameter(location: &str) -> RequestValidationRejection {
    parameter_problem(
        422,
        "https://openapi-to-rust.dev/problems/validation",
        "Request validation failed",
        "request_validation_failed",
        "required",
        location,
        "is required",
    )
}
fn schema_parameter_problem(
    location: &str,
    code: &str,
    message: &str,
) -> RequestValidationRejection {
    parameter_problem(
        422,
        "https://openapi-to-rust.dev/problems/validation",
        "Request validation failed",
        "request_validation_failed",
        code,
        location,
        message,
    )
}
fn parameter_problem(
    status: u16,
    problem_type: &str,
    title: &str,
    code: &str,
    error_code: &str,
    location: &str,
    message: &str,
) -> RequestValidationRejection {
    RequestValidationRejection(ProblemDetails {
        r#type: problem_type.to_string(),
        title: title.to_string(),
        status,
        code: code.to_string(),
        errors: vec![InvalidParameter {
            code: error_code.to_string(),
            location: location.to_string(),
            message: message.to_string(),
        }],
    })
}
pub(crate) fn request_body_too_large() -> RequestValidationRejection {
    public_problem(
        413,
        "https://openapi-to-rust.dev/problems/request-body-too-large",
        "Request body too large",
        "request_body_too_large",
    )
}
pub(crate) fn unsupported_media_type() -> RequestValidationRejection {
    public_problem(
        415,
        "https://openapi-to-rust.dev/problems/unsupported-media-type",
        "Unsupported media type",
        "unsupported_media_type",
    )
}
pub(crate) fn generated_contract_error() -> RequestValidationRejection {
    public_problem(
        500,
        "https://openapi-to-rust.dev/problems/generated-contract-error",
        "Internal server error",
        "generated_contract_error",
    )
}
fn public_problem(
    status: u16,
    problem_type: &str,
    title: &str,
    code: &str,
) -> RequestValidationRejection {
    RequestValidationRejection(ProblemDetails {
        r#type: problem_type.to_string(),
        title: title.to_string(),
        status,
        code: code.to_string(),
        errors: Vec::new(),
    })
}
fn public_violation(keyword: &str) -> (&'static str, &'static str) {
    match keyword {
        "required" => ("required", "is required"),
        "type" => ("type", "has an invalid type"),
        "enum" => ("enum", "has an unsupported value"),
        "const" => ("const", "has an unsupported value"),
        "format" => ("format", "has an invalid format"),
        "pattern" => ("pattern", "does not match the required format"),
        "minLength" => ("min_length", "does not meet the length constraint"),
        "maxLength" => ("max_length", "does not meet the length constraint"),
        "minimum" => ("minimum", "is outside the allowed range"),
        "maximum" => ("maximum", "is outside the allowed range"),
        "exclusiveMinimum" => ("exclusive_minimum", "is outside the allowed range"),
        "exclusiveMaximum" => ("exclusive_maximum", "is outside the allowed range"),
        "multipleOf" => ("multiple_of", "is outside the allowed range"),
        "minItems" => ("min_items", "does not meet the item-count constraint"),
        "maxItems" => ("max_items", "does not meet the item-count constraint"),
        "uniqueItems" => ("unique_items", "contains duplicate items"),
        "minProperties" => (
            "min_properties",
            "does not meet the property-count constraint",
        ),
        "maxProperties" => (
            "max_properties",
            "does not meet the property-count constraint",
        ),
        "additionalProperties" => ("additional_properties", "contains unsupported properties"),
        "unevaluatedProperties" => ("unevaluated_properties", "contains unsupported properties"),
        "anyOf" => ("any_of", "does not match the required shape"),
        "oneOf" => ("one_of", "does not match the required shape"),
        "not" => ("not", "does not match the required shape"),
        _ => ("invalid_value", "is invalid"),
    }
}
fn escape_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
