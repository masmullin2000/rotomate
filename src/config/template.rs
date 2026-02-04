//! Jinja2-style template rendering for YAML configuration values.

use anyhow::{Context, Result};
use minijinja::{Environment, UndefinedBehavior};
use std::collections::HashMap;

use crate::config::HostContext;

/// Built-in context variables available during execution-time template rendering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuiltinContext {
    /// Name of the host currently running the task.
    pub host: String,
    /// Name of the current task group.
    pub task_group: String,
    /// Timestamp in filesystem-safe format (YYYY-MM-DD_HH-MM-SS).
    pub timestamp: String,
}

/// Rendered task fields ready for execution.
#[derive(Debug, Clone)]
pub struct RenderedTaskFields {
    pub remote_command: Vec<String>,
    pub local_command: Vec<String>,
    pub upload: Vec<String>,
    pub download: Vec<String>,
}

/// Render task fields (commands, paths) with host context and builtin variables added to vars.
impl RenderedTaskFields {
    pub fn try_new(
        vars: &HashMap<String, serde_yaml::Value>,
        host_context: &HostContext,
        builtin: &BuiltinContext,
        remote_command: &[String],
        local_command: &[String],
        upload: &[String],
        download: &[String],
    ) -> Result<Self> {
        let env = Environment::build();

        // Build context with vars and host
        let mut context_map: HashMap<String, minijinja::Value> = vars
            .iter()
            .map(|(k, v)| (k.clone(), yaml_to_jinja_value(v)))
            .collect();

        // Add host context
        let host_value = minijinja::Value::from_serialize(host_context);
        context_map.insert("host".to_string(), host_value);

        // Add builtin context
        let builtin_value = minijinja::Value::from_serialize(builtin);
        context_map.insert("builtin".to_string(), builtin_value);

        let context = minijinja::Value::from_serialize(&context_map);

        // Render each field
        let remote_command = env.render_string_vec(remote_command, &context)?;
        let local_command = env.render_string_vec(local_command, &context)?;
        let upload = env.render_string_vec(upload, &context)?;
        let download = env.render_string_vec(download, &context)?;

        Ok(Self {
            remote_command,
            local_command,
            upload,
            download,
        })
    }
}

/// Extension trait for minijinja Environment with rendering helpers.
pub trait EnvironmentExt {
    /// Build a minijinja environment with custom functions.
    /// Uses strict undefined behavior (errors on undefined variables).
    fn build() -> Environment<'static>;

    /// Build a minijinja environment that allows undefined variables.
    /// Undefined variables are replaced with empty string.
    fn build_lenient() -> Environment<'static>;

    /// Render all string values in a YAML Value tree using the given context.
    fn render_yaml_value(
        &self,
        value: serde_yaml::Value,
        context: &minijinja::Value,
    ) -> Result<serde_yaml::Value>;

    /// Render a single string template with the given context.
    fn render_template(&self, template: &str, context: &minijinja::Value) -> Result<String>;

    /// Render a vector of strings.
    fn render_string_vec(
        &self,
        strings: &[String],
        context: &minijinja::Value,
    ) -> Result<Vec<String>>;
}

impl EnvironmentExt for Environment<'_> {
    fn build() -> Environment<'static> {
        build_environment_with_behavior(UndefinedBehavior::Strict)
    }

    fn build_lenient() -> Environment<'static> {
        build_environment_with_behavior(UndefinedBehavior::Lenient)
    }

    fn render_yaml_value(
        &self,
        value: serde_yaml::Value,
        context: &minijinja::Value,
    ) -> Result<serde_yaml::Value> {
        match value {
            serde_yaml::Value::String(s) => {
                let rendered = self.render_template(&s, context)?;
                Ok(serde_yaml::Value::String(rendered))
            }
            serde_yaml::Value::Sequence(seq) => {
                let rendered: Result<Vec<_>> = seq
                    .into_iter()
                    .map(|v| self.render_yaml_value(v, context))
                    .collect();
                Ok(serde_yaml::Value::Sequence(rendered?))
            }
            serde_yaml::Value::Mapping(map) => {
                let rendered: Result<serde_yaml::Mapping> = map
                    .into_iter()
                    .map(|(k, v)| {
                        let rendered_value = self.render_yaml_value(v, context)?;
                        Ok((k, rendered_value))
                    })
                    .collect();
                Ok(serde_yaml::Value::Mapping(rendered?))
            }
            // Pass through unchanged: Null, Bool, Number, Tagged
            other => Ok(other),
        }
    }

    fn render_template(&self, template: &str, context: &minijinja::Value) -> Result<String> {
        if !template.contains("{{") && !template.contains("{%") {
            return Ok(template.to_string());
        }

        self.render_str(template, context)
            .with_context(|| format!("Failed to render template: {template}"))
    }

    fn render_string_vec(
        &self,
        strings: &[String],
        context: &minijinja::Value,
    ) -> Result<Vec<String>> {
        strings
            .iter()
            .map(|s| self.render_template(s, context))
            .collect()
    }
}

fn build_environment_with_behavior(behavior: UndefinedBehavior) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(behavior);

    // env() function - get environment variable (empty if unset)
    env.add_function("env", |name: &str| -> String {
        std::env::var(name).unwrap_or_default()
    });

    // env_or() function - get environment variable with default value
    env.add_function("env_or", |name: &str, default: &str| -> String {
        std::env::var(name).unwrap_or_else(|_| default.to_string())
    });

    env
}

/// Convert YAML vars to minijinja Value context.
pub fn vars_to_context<H>(vars: &HashMap<String, serde_yaml::Value, H>) -> minijinja::Value
where
    H: std::hash::BuildHasher,
{
    let map: HashMap<String, minijinja::Value> = vars
        .iter()
        .map(|(k, v)| (k.clone(), yaml_to_jinja_value(v)))
        .collect();
    minijinja::Value::from_serialize(&map)
}

/// Convert YAML vars to minijinja Value context with placeholder objects for host and builtin.
/// This allows lenient mode to handle `host.*` and `builtin.*` attribute access without erroring.
pub fn vars_to_context_with_placeholders<H>(
    vars: &HashMap<String, serde_yaml::Value, H>,
) -> minijinja::Value
where
    H: std::hash::BuildHasher,
{
    let mut map: HashMap<String, minijinja::Value> = vars
        .iter()
        .map(|(k, v)| (k.clone(), yaml_to_jinja_value(v)))
        .collect();

    // Add empty placeholder objects for host and builtin contexts.
    // This allows attribute access (e.g., host.name, builtin.timestamp) to return
    // undefined values in lenient mode instead of erroring, so these templates
    // can be properly resolved at execution time.
    let empty_obj: HashMap<String, minijinja::Value> = HashMap::new();
    map.insert(
        "host".to_string(),
        minijinja::Value::from_serialize(&empty_obj),
    );
    map.insert(
        "builtin".to_string(),
        minijinja::Value::from_serialize(&empty_obj),
    );

    minijinja::Value::from_serialize(&map)
}

/// Convert a `serde_yaml::Value` to minijinja Value.
fn yaml_to_jinja_value(v: &serde_yaml::Value) -> minijinja::Value {
    match v {
        serde_yaml::Value::Null => minijinja::Value::UNDEFINED,
        serde_yaml::Value::Bool(b) => minijinja::Value::from(*b),
        serde_yaml::Value::Number(n) => n
            .as_i64()
            .map(minijinja::Value::from)
            .or_else(|| n.as_f64().map(minijinja::Value::from))
            .unwrap_or_else(|| minijinja::Value::from(n.to_string())),
        serde_yaml::Value::String(s) => minijinja::Value::from(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            let items: Vec<minijinja::Value> = seq.iter().map(yaml_to_jinja_value).collect();
            minijinja::Value::from(items)
        }
        serde_yaml::Value::Mapping(map) => {
            let items: HashMap<String, minijinja::Value> = map
                .iter()
                .filter_map(|(k, v)| {
                    k.as_str()
                        .map(|key| (key.to_string(), yaml_to_jinja_value(v)))
                })
                .collect();
            minijinja::Value::from_serialize(&items)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_jinja_value(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lenient_with_placeholders() {
        let env = Environment::build_lenient();

        // Use vars_to_context_with_placeholders which adds empty host and builtin objects
        let vars: HashMap<String, serde_yaml::Value> = HashMap::new();
        let context = vars_to_context_with_placeholders(&vars);

        // Test simple undefined - should render as empty in lenient mode
        let result = env.render_str("Hello {{ foo }}", &context);
        assert!(result.is_ok(), "Simple undefined should work: {:?}", result);
        assert_eq!(result.unwrap(), "Hello ");

        // Test attribute on builtin placeholder - should work now
        let result = env.render_str("Hello {{ builtin.host }}", &context);
        assert!(
            result.is_ok(),
            "builtin.host should work with placeholder: {:?}",
            result
        );
        assert_eq!(result.unwrap(), "Hello ");

        // Test attribute on host placeholder
        let result = env.render_str("Host: {{ host.name }}", &context);
        assert!(
            result.is_ok(),
            "host.name should work with placeholder: {:?}",
            result
        );
        assert_eq!(result.unwrap(), "Host: ");

        // Test nested attribute access
        let result = env.render_str("{{ builtin.timestamp }}-{{ host.hostname }}", &context);
        assert!(
            result.is_ok(),
            "Multiple placeholders should work: {:?}",
            result
        );
        assert_eq!(result.unwrap(), "-");
    }
}
