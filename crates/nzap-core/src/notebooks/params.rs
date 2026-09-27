//! The notebook parameter contract (port of hosted NZAP's
//! `server/src/notebooks/schema.ts`).
//!
//! A notebook declares its parameters; submitted values are validated
//! against that declaration and injected as a `params` dict ahead of the
//! source. The contract a notebook author codes against is just:
//!
//! ```python
//! print(params["string_to_print"])
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::python;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Text,
    Integer,
    Number,
    Boolean,
    Select,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookParam {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: ParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default)]
    pub required: bool,
    /// Only for `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A valid Python identifier of at most 63 characters.
pub fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    key.len() <= 63
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validate a parameter declaration (the editor and the public catalog).
pub fn validate(params: &[NotebookParam]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for param in params {
        if !valid_key(&param.key) {
            return Err(Error::invalid(format!(
                "Parameter key '{}' must be a valid Python identifier.",
                param.key
            )));
        }
        if !seen.insert(param.key.as_str()) {
            return Err(Error::invalid(format!("Parameter key '{}' is declared twice.", param.key)));
        }
        if param.label.trim().is_empty() {
            return Err(Error::invalid(format!("Parameter '{}' needs a label.", param.key)));
        }
        if param.kind == ParamType::Select && param.options.as_ref().is_none_or(Vec::is_empty) {
            return Err(Error::invalid(format!(
                "Select parameter '{}' needs at least one option.",
                param.key
            )));
        }
        if let Some(default) = &param.default {
            if !(default.is_null() || default.is_string() || default.is_number() || default.is_boolean()) {
                return Err(Error::invalid(format!(
                    "The default of '{}' must be a string, number or boolean.",
                    param.key
                )));
            }
        }
    }
    Ok(())
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Validate submitted values against the declaration and return the
/// coerced `params` dict. Empty submissions fall back to defaults.
pub fn resolve(declared: &[NotebookParam], submitted: &Map<String, Value>) -> Result<Map<String, Value>> {
    let mut resolved = Map::new();
    for param in declared {
        let raw = submitted.get(&param.key).filter(|value| {
            !value.is_null() && value.as_str().is_none_or(|text| !text.is_empty())
        });
        let Some(value) = raw.or(param.default.as_ref()).filter(|value| !value.is_null()) else {
            if param.required {
                return Err(Error::invalid(format!("{} is required.", param.label)));
            }
            continue;
        };
        let coerced = match param.kind {
            ParamType::String | ParamType::Text => Value::String(as_text(value)),
            ParamType::Integer => {
                let number = as_number(value)
                    .filter(|number| number.is_finite() && number.fract() == 0.0 && number.abs() < 9e15)
                    .ok_or_else(|| Error::invalid(format!("{} must be a whole number.", param.label)))?;
                Value::from(number as i64)
            }
            ParamType::Number => {
                let number = as_number(value)
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| Error::invalid(format!("{} must be a number.", param.label)))?;
                serde_json::Number::from_f64(number).map(Value::Number).unwrap_or(Value::Null)
            }
            ParamType::Boolean => Value::Bool(matches!(value, Value::Bool(true)) || matches!(value.as_str(), Some("true" | "on"))),
            ParamType::Select => {
                let text = as_text(value);
                if param.options.as_ref().is_some_and(|options| !options.contains(&text)) {
                    return Err(Error::invalid(format!(
                        "{} must be one of the offered options.",
                        param.label
                    )));
                }
                Value::String(text)
            }
        };
        resolved.insert(param.key.clone(), coerced);
    }
    Ok(resolved)
}

/// What the kernel actually runs: the params dict, then the notebook source.
/// The dict travels as JSON inside a Python string literal and is decoded
/// with `json.loads`, so every value — strings, numbers, booleans — arrives
/// as the right Python type.
pub fn assemble_source(source: &str, params: &Map<String, Value>) -> String {
    let encoded = Value::Object(params.clone()).to_string();
    format!(
        "# Injected by NZAP Engine — do not edit.\n\
         import json as _nzap_json\n\
         params = _nzap_json.loads({})\n\
         del _nzap_json\n\n{source}",
        python::literal(&encoded)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(value: Value) -> Vec<NotebookParam> {
        serde_json::from_value(value).unwrap()
    }

    fn submitted(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn declarations_are_validated() {
        assert!(validate(&params(json!([{"key": "a_1", "label": "A", "type": "string"}]))).is_ok());
        for bad in [
            json!([{"key": "1a", "label": "A", "type": "string"}]),
            json!([{"key": "a", "label": " ", "type": "string"}]),
            json!([{"key": "a", "label": "A", "type": "select"}]),
            json!([{"key": "a", "label": "A", "type": "select", "options": []}]),
            json!([{"key": "a", "label": "A", "type": "string", "default": [1]}]),
            json!([{"key": "a", "label": "A", "type": "string"}, {"key": "a", "label": "B", "type": "text"}]),
        ] {
            assert!(validate(&params(bad.clone())).is_err(), "{bad}");
        }
        assert!(serde_json::from_value::<Vec<NotebookParam>>(json!([{"key": "a", "label": "A", "type": "file"}])).is_err());
    }

    #[test]
    fn values_are_coerced_like_hosted_nzap() {
        let declared = params(json!([
            {"key": "s", "label": "S", "type": "string", "default": "hi", "required": true},
            {"key": "i", "label": "I", "type": "integer"},
            {"key": "n", "label": "N", "type": "number"},
            {"key": "b", "label": "B", "type": "boolean"},
            {"key": "c", "label": "C", "type": "select", "options": ["x", "y"], "default": "x"},
            {"key": "t", "label": "T", "type": "text"},
        ]));
        let resolved = resolve(
            &declared,
            &submitted(json!({"s": "", "i": "42", "n": 2.5, "b": "on", "c": "y", "extra": 1})),
        )
        .unwrap();
        assert_eq!(
            Value::Object(resolved),
            json!({"s": "hi", "i": 42, "n": 2.5, "b": true, "c": "y"})
        );

        let err = |value: Value| resolve(&declared, &submitted(value)).unwrap_err().to_string();
        assert_eq!(err(json!({"i": "4.5"})), "I must be a whole number.");
        assert_eq!(err(json!({"n": "abc"})), "N must be a number.");
        assert_eq!(err(json!({"c": "z"})), "C must be one of the offered options.");
        let required = params(json!([{"key": "k", "label": "Key", "type": "string", "required": true}]));
        assert_eq!(resolve(&required, &Map::new()).unwrap_err().to_string(), "Key is required.");
        assert_eq!(
            Value::Object(resolve(&params(json!([{"key": "b", "label": "B", "type": "boolean"}])), &submitted(json!({"b": false}))).unwrap()),
            json!({"b": false})
        );
    }

    #[test]
    fn source_injection_survives_hostile_values() {
        let mut values = Map::new();
        values.insert("s".into(), json!("\"); import os; os.system('x') #\n'''"));
        values.insert("flag".into(), json!(true));
        let code = assemble_source("print(params['s'])", &values);
        let lines: Vec<&str> = code.lines().collect();
        assert_eq!(lines[0], "# Injected by NZAP Engine — do not edit.");
        assert_eq!(lines[1], "import json as _nzap_json");
        assert!(lines[2].starts_with("params = _nzap_json.loads(\""));
        assert!(lines[2].ends_with("\")"));
        assert_eq!(lines[3], "del _nzap_json");
        assert_eq!(lines.last(), Some(&"print(params['s'])"));
        // The whole dict is one string literal on one line.
        let literal = lines[2].trim_start_matches("params = _nzap_json.loads(").trim_end_matches(')');
        let decoded: String = serde_json::from_str(literal).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&decoded).unwrap(), Value::Object(values));
    }
}
