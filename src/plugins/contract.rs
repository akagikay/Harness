use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fmt;

use crate::config::PermissionLevel;

/// High-level permission category declared by a tool's contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    #[serde(alias = "ReadOnly", alias = "readonly")]
    ReadOnly,
    #[serde(alias = "WorkspaceWrite", alias = "workspacewrite")]
    WorkspaceWrite,
    #[serde(alias = "ShellOrNetwork", alias = "shellornetwork")]
    ShellOrNetwork,
    #[serde(alias = "HighRisk", alias = "highrisk")]
    HighRisk,
    #[serde(alias = "Meta")]
    Meta,
}

impl Default for ToolPermission {
    fn default() -> Self {
        ToolPermission::ReadOnly
    }
}

impl ToolPermission {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolPermission::ReadOnly => "read_only",
            ToolPermission::WorkspaceWrite => "workspace_write",
            ToolPermission::ShellOrNetwork => "shell_or_network",
            ToolPermission::HighRisk => "high_risk",
            ToolPermission::Meta => "meta",
        }
    }

    /// Default harness permission policy when no explicit rule or mode override applies.
    pub fn default_policy_level(&self) -> PermissionLevel {
        match self {
            ToolPermission::ReadOnly | ToolPermission::Meta => PermissionLevel::Allow,
            ToolPermission::WorkspaceWrite
            | ToolPermission::ShellOrNetwork
            | ToolPermission::HighRisk => PermissionLevel::Ask,
        }
    }
}

impl fmt::Display for ToolPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// JSON value type expected for an input field in a tool contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    #[serde(alias = "String", alias = "str")]
    String,
    #[serde(alias = "Integer", alias = "int")]
    Integer,
    #[serde(alias = "Number", alias = "float")]
    Number,
    #[serde(alias = "Boolean", alias = "bool")]
    Boolean,
    #[serde(alias = "Array")]
    Array,
    #[serde(alias = "Object")]
    Object,
}

impl FieldType {
    pub fn as_json_type(&self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Integer => "integer",
            FieldType::Number => "number",
            FieldType::Boolean => "boolean",
            FieldType::Array => "array",
            FieldType::Object => "object",
        }
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_json_type())
    }
}

/// Specification and validation rules for a single tool parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFieldSpec {
    pub name: String,
    pub field_type: FieldType,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default)]
    pub non_empty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub error_hint: String,
}

impl ToolFieldSpec {
    fn new(
        name: impl Into<String>,
        field_type: FieldType,
        description: impl Into<String>,
        required: bool,
        error_hint: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            field_type,
            description: description.into(),
            required,
            default: None,
            enum_values: None,
            non_empty: false,
            min: None,
            max: None,
            error_hint: error_hint.into(),
        }
    }

    pub fn required_string(
        name: impl Into<String>,
        description: impl Into<String>,
        error_hint: impl Into<String>,
    ) -> Self {
        Self::new(name, FieldType::String, description, true, error_hint)
    }

    pub fn optional_string(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, FieldType::String, description, false, "")
    }

    pub fn required_int(
        name: impl Into<String>,
        description: impl Into<String>,
        error_hint: impl Into<String>,
    ) -> Self {
        Self::new(name, FieldType::Integer, description, true, error_hint)
    }

    pub fn optional_int(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, FieldType::Integer, description, false, "")
    }

    pub fn required_number(
        name: impl Into<String>,
        description: impl Into<String>,
        error_hint: impl Into<String>,
    ) -> Self {
        Self::new(name, FieldType::Number, description, true, error_hint)
    }

    pub fn optional_number(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, FieldType::Number, description, false, "")
    }

    pub fn required_bool(
        name: impl Into<String>,
        description: impl Into<String>,
        error_hint: impl Into<String>,
    ) -> Self {
        Self::new(name, FieldType::Boolean, description, true, error_hint)
    }

    pub fn optional_bool(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, FieldType::Boolean, description, false, "")
    }

    pub fn required_array(
        name: impl Into<String>,
        description: impl Into<String>,
        error_hint: impl Into<String>,
    ) -> Self {
        Self::new(name, FieldType::Array, description, true, error_hint)
    }

    pub fn optional_array(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, FieldType::Array, description, false, "")
    }

    pub fn required_object(
        name: impl Into<String>,
        description: impl Into<String>,
        error_hint: impl Into<String>,
    ) -> Self {
        Self::new(name, FieldType::Object, description, true, error_hint)
    }

    pub fn optional_object(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, FieldType::Object, description, false, "")
    }

    pub fn with_default(mut self, default: Value) -> Self {
        self.default = Some(default);
        self
    }

    pub fn with_enum<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.enum_values = Some(values.into_iter().map(Into::into).collect());
        self
    }

    pub fn non_empty(mut self) -> Self {
        self.non_empty = true;
        self
    }

    pub fn with_min(mut self, min: f64) -> Self {
        self.min = Some(min);
        self
    }

    pub fn with_max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }

    pub fn with_range(mut self, min: Option<f64>, max: Option<f64>) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn with_error_hint(mut self, hint: impl Into<String>) -> Self {
        self.error_hint = hint.into();
        self
    }

    /// Render this field as a JSON Schema property object.
    pub fn to_schema_property(&self) -> Value {
        let mut prop = Map::new();
        prop.insert("type".to_string(), json!(self.field_type.as_json_type()));
        prop.insert("description".to_string(), json!(self.description));

        if let Some(ref enums) = self.enum_values {
            prop.insert("enum".to_string(), json!(enums));
        }
        if let Some(ref def) = self.default {
            prop.insert("default".to_string(), def.clone());
        }
        if matches!(self.field_type, FieldType::Integer | FieldType::Number) {
            if let Some(min) = self.min {
                prop.insert("minimum".to_string(), json!(min));
            }
            if let Some(max) = self.max {
                prop.insert("maximum".to_string(), json!(max));
            }
        }
        if self.field_type == FieldType::Array {
            // OpenAI-compatible function schemas require `items` on array properties.
            prop.insert("items".to_string(), json!({}));
        }

        Value::Object(prop)
    }

    fn format_hint_suffix(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref allowed) = self.enum_values {
            parts.push(format!("allowed values: [{}]", allowed.join(", ")));
        }
        if self.min.is_some() || self.max.is_some() {
            let bounds = match (self.min, self.max) {
                (Some(lo), Some(hi)) => format!("range: {}..={}", lo, hi),
                (Some(lo), None) => format!("min: {}", lo),
                (None, Some(hi)) => format!("max: {}", hi),
                (None, None) => String::new(),
            };
            if !bounds.is_empty() {
                parts.push(bounds);
            }
        }
        if self.non_empty {
            parts.push("must be non-empty".to_string());
        }
        if !self.error_hint.trim().is_empty() {
            parts.push(format!("hint: {}", self.error_hint.trim()));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join("; "))
        }
    }

    fn validate_value(&self, tool_name: &str, value: Option<&Value>) -> Result<(), String> {
        let hint_suffix = self.format_hint_suffix();

        let val = match value {
            None | Some(Value::Null) => {
                if self.required {
                    return Err(format!(
                        "Missing required field '{}' (expected type: {}) for tool '{}'{}",
                        self.name, self.field_type, tool_name, hint_suffix
                    ));
                }
                return Ok(());
            }
            Some(v) => v,
        };

        match self.field_type {
            FieldType::String => {
                let s = val.as_str().ok_or_else(|| {
                    format!(
                        "Field '{}' on tool '{}' must be a string, got {}{}",
                        self.name,
                        tool_name,
                        json_type_name(val),
                        hint_suffix
                    )
                })?;
                if self.non_empty && s.trim().is_empty() {
                    return Err(format!(
                        "Field '{}' on tool '{}' must be a non-empty string{}",
                        self.name, tool_name, hint_suffix
                    ));
                }
                if let Some(ref allowed) = self.enum_values {
                    if !allowed.iter().any(|candidate| candidate == s) {
                        return Err(format!(
                            "Invalid value '{}' for field '{}' on tool '{}' (expected one of [{}]){}",
                            s,
                            self.name,
                            tool_name,
                            allowed.join(", "),
                            hint_suffix
                        ));
                    }
                }
            }
            FieldType::Integer => {
                if !(val.is_i64() || val.is_u64()) {
                    return Err(format!(
                        "Field '{}' on tool '{}' must be an integer, got {}{}",
                        self.name,
                        tool_name,
                        json_type_name(val),
                        hint_suffix
                    ));
                }
                let num = val.as_f64().unwrap_or(0.0);
                self.validate_numeric_bounds(tool_name, num, &hint_suffix)?;
            }
            FieldType::Number => {
                let num = val.as_f64().ok_or_else(|| {
                    format!(
                        "Field '{}' on tool '{}' must be a number, got {}{}",
                        self.name,
                        tool_name,
                        json_type_name(val),
                        hint_suffix
                    )
                })?;
                self.validate_numeric_bounds(tool_name, num, &hint_suffix)?;
            }
            FieldType::Boolean => {
                if !val.is_boolean() {
                    return Err(format!(
                        "Field '{}' on tool '{}' must be a boolean, got {}{}",
                        self.name,
                        tool_name,
                        json_type_name(val),
                        hint_suffix
                    ));
                }
            }
            FieldType::Array => {
                let arr = val.as_array().ok_or_else(|| {
                    format!(
                        "Field '{}' on tool '{}' must be an array, got {}{}",
                        self.name,
                        tool_name,
                        json_type_name(val),
                        hint_suffix
                    )
                })?;
                if self.non_empty && arr.is_empty() {
                    return Err(format!(
                        "Field '{}' on tool '{}' must be a non-empty array{}",
                        self.name, tool_name, hint_suffix
                    ));
                }
            }
            FieldType::Object => {
                let obj = val.as_object().ok_or_else(|| {
                    format!(
                        "Field '{}' on tool '{}' must be an object, got {}{}",
                        self.name,
                        tool_name,
                        json_type_name(val),
                        hint_suffix
                    )
                })?;
                if self.non_empty && obj.is_empty() {
                    return Err(format!(
                        "Field '{}' on tool '{}' must be a non-empty object{}",
                        self.name, tool_name, hint_suffix
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_numeric_bounds(
        &self,
        tool_name: &str,
        num: f64,
        hint_suffix: &str,
    ) -> Result<(), String> {
        if !num.is_finite() {
            return Err(format!(
                "Field '{}' on tool '{}' must be a finite number{}",
                self.name, tool_name, hint_suffix
            ));
        }
        if let Some(min) = self.min {
            if num < min {
                return Err(format!(
                    "Field '{}' on tool '{}' value {} is below minimum {}{}",
                    self.name, tool_name, num, min, hint_suffix
                ));
            }
        }
        if let Some(max) = self.max {
            if num > max {
                return Err(format!(
                    "Field '{}' on tool '{}' value {} exceeds maximum {}{}",
                    self.name, tool_name, num, max, hint_suffix
                ));
            }
        }
        Ok(())
    }
}

/// Expected output format and example documented by a tool contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContract {
    #[serde(default = "default_output_format")]
    pub format: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub example: String,
}

fn default_output_format() -> String {
    "text".to_string()
}

impl Default for OutputContract {
    fn default() -> Self {
        Self {
            format: default_output_format(),
            description: String::new(),
            example: String::new(),
        }
    }
}

impl OutputContract {
    pub fn new(
        format: impl Into<String>,
        description: impl Into<String>,
        example: impl Into<String>,
    ) -> Self {
        Self {
            format: format.into(),
            description: description.into(),
            example: example.into(),
        }
    }
}

/// Explicit failure mode documented on a tool contract for agent self-correction and UI inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFailureMode {
    pub code: String,
    pub condition: String,
    pub error_message: String,
    #[serde(default)]
    pub recovery_hint: String,
}

impl ToolFailureMode {
    pub fn new(
        code: impl Into<String>,
        condition: impl Into<String>,
        error_message: impl Into<String>,
        recovery_hint: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            condition: condition.into(),
            error_message: error_message.into(),
            recovery_hint: recovery_hint.into(),
        }
    }
}

fn default_timeout_secs() -> u64 {
    30
}

/// First-class modular contract describing a tool's identity, permissions, inputs, outputs, and failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContract {
    pub name: String,
    pub description: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub permission_level: ToolPermission,
    #[serde(default)]
    pub fields: Vec<ToolFieldSpec>,
    #[serde(default)]
    pub expected_output: OutputContract,
    #[serde(default)]
    pub failure_modes: Vec<ToolFailureMode>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub is_custom: bool,
}

fn default_category() -> String {
    "general".to_string()
}

impl ToolContract {
    /// Generates an OpenAI-compatible JSON Schema object from `self.fields`.
    pub fn to_json_schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();

        for field in &self.fields {
            properties.insert(field.name.clone(), field.to_schema_property());
            if field.required {
                required.push(Value::String(field.name.clone()));
            }
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    /// Validates tool call arguments against every declared field constraint.
    pub fn validate_args(&self, args: &Value) -> Result<(), String> {
        let map = args.as_object().ok_or_else(|| {
            let expected: Vec<String> = self
                .fields
                .iter()
                .map(|f| {
                    if f.required {
                        format!("{}* ({})", f.name, f.field_type)
                    } else {
                        format!("{} ({})", f.name, f.field_type)
                    }
                })
                .collect();
            format!(
                "Contract violation for tool '{}': arguments must be a JSON object, got {}. Expected fields: [{}]",
                self.name,
                json_type_name(args),
                expected.join(", ")
            )
        })?;

        let mut errors = Vec::new();
        for field in &self.fields {
            if let Err(err) = field.validate_value(&self.name, map.get(&field.name)) {
                errors.push(err);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else if errors.len() == 1 {
            Err(errors.into_iter().next().unwrap())
        } else {
            Err(format!(
                "Contract validation failed for tool '{}':\n- {}",
                self.name,
                errors.join("\n- ")
            ))
        }
    }
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
