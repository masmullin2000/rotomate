use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::schema;
use super::template::{EnvironmentExt, vars_to_context, vars_to_context_with_placeholders};

/// Load and merge multiple YAML configuration files.
/// Files are merged in order, with later files overriding earlier ones.
pub fn load_configs<P: AsRef<Path>>(paths: &[P]) -> Result<schema::Config> {
    paths
        .iter()
        .try_fold(schema::Config::default(), |mut acc, path| {
            acc.merge(load_config(path)?);
            Ok(acc)
        })
}

/// Load and parse a YAML configuration file, recursively processing imports.
fn load_config<P: AsRef<Path>>(path: P) -> Result<schema::Config> {
    let mut visited = HashSet::new();
    load_config_recursive(path.as_ref(), &mut visited, &HashMap::new())
}

/// Internal recursive loader that tracks visited files to prevent cycles.
fn load_config_recursive(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    inherited_vars: &HashMap<String, serde_yaml::Value>,
) -> Result<schema::Config> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("Failed to resolve config path: {}", path.display()))?;

    // Skip circular imports (file already processed)
    if visited.contains(&canonical) {
        log::debug!("Skipping circular import: {}", path.display());
        return Ok(schema::Config::default());
    }
    visited.insert(canonical);

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    // Parse as raw YAML Value first
    let mut raw_value: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
        let msg = e.to_string();
        if let Some(hint) = diagnose_yaml_parse_error(&content, &msg) {
            return anyhow::anyhow!("\n  {}: {hint}", path.display());
        }
        anyhow::anyhow!("Failed to parse config file: {}\n  {msg}", path.display())
    })?;

    // Extract imports and vars before rendering (they shouldn't be templated)
    let imports = extract_imports(&raw_value);
    let file_vars = extract_vars(&raw_value);

    // Merge file's own vars on top of inherited vars BEFORE processing imports
    // so that imported files can use vars defined in the importing file
    let mut merged_vars = inherited_vars.clone();
    merged_vars.extend(file_vars);

    // Process imports with the merged vars
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let (mut merged_vars, mut merged_config) = imports.iter().try_fold(
        (merged_vars, schema::Config::default()),
        |(mut vars, mut config), import_path| {
            let full_path = if import_path.is_absolute() {
                import_path.clone()
            } else {
                base_dir.join(import_path)
            };

            let expanded_path = expand_tilde(&full_path);
            let imported = load_config_recursive(&expanded_path, visited, &vars)?;

            vars.extend(imported.vars.clone());
            config.merge(imported);
            anyhow::Ok((vars, config))
        },
    )?;

    // Render templates in the YAML value tree
    // Use lenient mode to allow undefined host.* variables (resolved at execution time)
    let env = minijinja::Environment::build_lenient();

    // First, render vars that reference other vars (iteratively until stable)
    merged_vars = render_vars(&env, merged_vars)?;

    let context = vars_to_context_with_placeholders(&merged_vars);

    // Selectively render YAML, skipping task command fields that need execution-time variables
    // (host.*, builtin.*). These fields are rendered later in the executor.
    raw_value = render_yaml_selective(&env, raw_value, &context)
        .with_context(|| format!("Failed to render templates in: {}", path.display()))?;

    // Deserialize the rendered value into schema::Config
    let mut config: schema::Config = serde_yaml::from_value(raw_value).map_err(|e| {
        let msg = e.to_string();
        let line_hint = find_error_line(&content, &msg);
        match line_hint {
            Some(line_num) => anyhow::anyhow!(
                "\n  {}:{}: {msg}",
                path.display(),
                line_num
            ),
            None => anyhow::anyhow!(
                "\n  {}: {msg}",
                path.display()
            ),
        }
    })?;

    // Store the merged vars in config for later use (executor needs them)
    config.vars.clone_from(&merged_vars);

    // Expand ~ in paths
    expand_paths(&mut config);

    // Clear imports (they've been processed)
    config.imports.clear();

    // Track source file for all items defined in this file
    let source_path = path.to_path_buf();
    for key in config.hosts.keys() {
        config.hosts_source.insert(key.clone(), source_path.clone());
    }
    for key in config.tasks.keys() {
        config.tasks_source.insert(key.clone(), source_path.clone());
    }
    for key in config.task_groups.keys() {
        config
            .groups_source
            .insert(key.clone(), source_path.clone());
    }
    for key in config.campaigns.keys() {
        config
            .campaigns_source
            .insert(key.clone(), source_path.clone());
    }

    // Merge current config on top of imported configs
    merged_config.merge(config);

    Ok(merged_config)
}

/// Keys within tasks that should NOT be rendered at config load time.
/// These fields may contain `host.*` and `builtin.*` variables that are only
/// available at execution time.
const TASK_DEFERRED_KEYS: &[&str] = &[
    "remote_command",
    "local_command",
    "upload",
    "download",
    "delete_remote",
    "delete_local",
    "steps",
];

/// Render YAML values selectively, skipping task command fields.
/// Task command fields (`remote_command`, `local_command`, `upload`, `download`) are
/// deferred to execution time where `host.*` and `builtin.*` context is available.
fn render_yaml_selective(
    env: &minijinja::Environment,
    value: serde_yaml::Value,
    context: &minijinja::Value,
) -> Result<serde_yaml::Value> {
    render_yaml_recursive(env, value, context, &[])
}

/// Recursively render YAML, tracking the current path to skip deferred fields.
fn render_yaml_recursive(
    env: &minijinja::Environment,
    value: serde_yaml::Value,
    context: &minijinja::Value,
    path: &[&str],
) -> Result<serde_yaml::Value> {
    // Check if we're inside a task and at a deferred key
    if path.len() >= 3 && path.first() == Some(&"tasks") && TASK_DEFERRED_KEYS.contains(&path[2]) {
        // If at a deferred key, return value unchanged (will be rendered at execution time)
        return Ok(value);
    }

    match value {
        serde_yaml::Value::String(s) => {
            // Only render if contains template syntax
            if !s.contains("{{") && !s.contains("{%") {
                return Ok(serde_yaml::Value::String(s));
            }
            let rendered = env
                .render_str(&s, context)
                .with_context(|| format!("Failed to render template: {s}"))?;
            Ok(serde_yaml::Value::String(rendered))
        }
        serde_yaml::Value::Sequence(seq) => {
            let rendered: Result<Vec<_>> = seq
                .into_iter()
                .map(|v| render_yaml_recursive(env, v, context, path))
                .collect();
            Ok(serde_yaml::Value::Sequence(rendered?))
        }
        serde_yaml::Value::Mapping(map) => {
            let rendered: Result<serde_yaml::Mapping> = map
                .into_iter()
                .map(|(k, v)| {
                    let key_str = k.as_str().unwrap_or("");
                    let mut new_path = path.to_vec();
                    new_path.push(key_str);
                    let rendered_value = render_yaml_recursive(env, v, context, &new_path)?;
                    Ok((k, rendered_value))
                })
                .collect();
            Ok(serde_yaml::Value::Mapping(rendered?))
        }
        // Pass through unchanged: Null, Bool, Number, Tagged
        other => Ok(other),
    }
}

/// Render vars that reference other vars, iteratively until stable.
fn render_vars(
    env: &minijinja::Environment,
    mut vars: HashMap<String, serde_yaml::Value>,
) -> Result<HashMap<String, serde_yaml::Value>> {
    const MAX_ITERATIONS: usize = 10;

    for _ in 0..MAX_ITERATIONS {
        let context = vars_to_context(&vars);
        let mut changed = false;

        let rendered = vars
            .iter()
            .map(|(k, v)| {
                let rendered_value = env.render_yaml_value(v.clone(), &context)?;
                if rendered_value != *v {
                    changed = true;
                }
                Ok((k.clone(), rendered_value))
            })
            .collect::<Result<_>>()?;

        vars = rendered;

        if !changed {
            break;
        }
    }

    Ok(vars)
}

/// Extract imports from raw YAML value without rendering.
fn extract_imports(value: &serde_yaml::Value) -> Vec<PathBuf> {
    value
        .get("imports")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Extract vars from raw YAML value without rendering.
fn extract_vars(value: &serde_yaml::Value) -> HashMap<String, serde_yaml::Value> {
    value
        .get("vars")
        .and_then(|v| v.as_mapping())
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| k.as_str().map(|key| (key.to_string(), v.clone())))
                .collect()
        })
        .unwrap_or_default()
}

/// Expand ~ to home directory in all path fields.
fn expand_paths(config: &mut schema::Config) {
    if let Some(ref mut key) = config.defaults.private_key {
        *key = expand_tilde(key);
    }

    for host in config.hosts.values_mut() {
        if let Some(ref mut key) = host.private_key {
            *key = expand_tilde(key);
        }
    }
}

/// Expand ~ to home directory in a path.
fn expand_tilde(path: &Path) -> std::path::PathBuf {
    if let Some(path_str) = path.to_str()
        && let Ok(expanded) = shellexpand::full(path_str)
    {
        return std::path::PathBuf::from(expanded.as_ref());
    }
    path.to_path_buf()
}

/// Diagnose a YAML parse error and return an actionable message if possible.
///
/// Detects common issues like unquoted `{{ }}` template expressions that YAML
/// interprets as flow mapping syntax (`{key: value}`).
fn diagnose_yaml_parse_error(content: &str, error_msg: &str) -> Option<String> {
    // Extract line number from serde_yaml error (format: "... at line N column M")
    let line_num = extract_error_line_number(error_msg)?;
    let line = content.lines().nth(line_num - 1)?;

    // Check if the line contains template syntax that YAML misinterprets
    if line.contains("{{") || line.contains("}}") {
        let trimmed = line.trim();
        // Strip leading "- " list marker for the suggested fix
        let value = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        Some(format!(
            "line {line_num}: YAML is interpreting `{{{{ }}}}` as flow mapping syntax\n\
             \n  {trimmed}\n\n  \
             Suggested fix: wrap the value in quotes:\n  \
             - '{value}'"
        ))
    } else {
        None
    }
}

/// Extract line number from a serde_yaml error message.
/// Matches patterns like "at line 21 column 58".
fn extract_error_line_number(error_msg: &str) -> Option<usize> {
    let marker = "at line ";
    let start = error_msg.find(marker)?;
    let rest = &error_msg[start + marker.len()..];
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

/// Try to find the line number for a deserialization error by searching the raw YAML content.
///
/// The custom `ExecCmdline` deserializer includes the reconstructed command in its error
/// message in the format: `command must be quoted:\n  - 'COMMAND'`
/// We extract the part before the first `: ` in the command (the YAML mapping key)
/// and search for it in the raw content.
fn find_error_line(content: &str, error_msg: &str) -> Option<usize> {
    // Extract text between "- '" and trailing "'"
    let start = error_msg.find("- '")?;
    let inner = &error_msg[start + 3..];
    let end = inner.rfind('\'')?;
    let command = &inner[..end];

    // The key is the part before the first `: ` — this is what YAML split on
    let search_key = command.split(": ").next()?.trim();
    if search_key.is_empty() {
        return None;
    }

    for (i, line) in content.lines().enumerate() {
        if line.contains(search_key) {
            return Some(i + 1);
        }
    }
    None
}
