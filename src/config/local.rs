use std::path::Path;

use anyhow::{Context, Result};
use serde_yaml_ng::Value;

use super::Config;

pub(super) fn apply(path: &Path, base: &str, config: Config) -> Result<Config> {
    let local_path = path.with_file_name("cryptty.local.yaml");
    let text = match std::fs::read_to_string(&local_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(config);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Cannot read local config {}", local_path.display())
            });
        }
    };
    Config::parse(&text).with_context(|| {
        format!("Invalid local config {}", local_path.display())
    })?;
    let local: Value = serde_yaml_ng::from_str(&text)?;
    if local.is_null() {
        return Ok(config);
    }
    let mut merged: Value = serde_yaml_ng::from_str(base)?;
    merge(&mut merged, local);
    serde_yaml_ng::from_value::<Option<Config>>(merged)
        .map(Option::unwrap_or_default)
        .with_context(|| {
            format!(
                "Invalid config after applying {} to {}",
                local_path.display(),
                path.display(),
            )
        })
}

fn merge(base: &mut Value, local: Value) {
    match (base, local) {
        (Value::Mapping(base), Value::Mapping(local)) => {
            for (key, value) in local {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, local) => *base = local,
    }
}
