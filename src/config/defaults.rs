use std::collections::BTreeMap;

use anyhow::Result;
use serde_yaml_ng::{Mapping, Value};

pub(super) fn additions(text: &str) -> Result<String> {
    let defaults =
        serde_yaml_ng::from_str(include_str!("../default-config.yaml"))?;
    append_missing(text, &defaults)
}

fn append_missing(text: &str, defaults: &Value) -> Result<String> {
    let existing = serde_yaml_ng::from_str::<Value>(text)?;
    let comments = commented_fields(text);
    let Some(missing) = missing(defaults, Some(&existing), &[], &comments)
    else {
        return Ok(String::new());
    };
    let mut output = String::new();
    if !text.is_empty() && !text.ends_with('\n') {
        output.push('\n');
    }
    for line in serde_yaml_ng::to_string(&missing)?.lines() {
        output.push_str("# ");
        output.push_str(line);
        output.push('\n');
    }
    Ok(output)
}

fn commented_fields(text: &str) -> BTreeMap<Vec<String>, Value> {
    let mut fields = BTreeMap::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    for line in text.lines() {
        let (yaml, commented) = match line.trim_start().strip_prefix("# ") {
            Some(comment) => (comment, true),
            None => (line, false),
        };
        let Ok(mapping) = serde_yaml_ng::from_str::<Mapping>(yaml) else {
            continue;
        };
        if mapping.len() != 1 {
            continue;
        }
        let indent = line.len() - line.trim_start().len()
            + if commented {
                yaml.len() - yaml.trim_start().len()
            } else {
                0
            };
        while parents.last().is_some_and(|(level, _)| *level >= indent) {
            parents.pop();
        }
        let (key, value) = mapping.into_iter().next().unwrap();
        let Some(key) = key.as_str() else { continue };
        if value.is_null() {
            parents.push((indent, key.to_string()));
        } else if commented {
            let mut path: Vec<_> =
                parents.iter().map(|(_, k)| k.clone()).collect();
            path.push(key.to_string());
            fields.insert(path, value);
        }
    }
    fields
}

fn missing(
    defaults: &Value,
    existing: Option<&Value>,
    path: &[String],
    comments: &BTreeMap<Vec<String>, Value>,
) -> Option<Value> {
    if let Value::Mapping(fields) = defaults {
        let mut result = Mapping::new();
        for (key, value) in fields {
            let mut path = path.to_vec();
            path.push(
                key.as_str().expect("Default keys are strings").to_string(),
            );
            if let Some(value) = missing(
                value,
                existing.and_then(|v| v.get(key)),
                &path,
                comments,
            ) {
                result.insert(key.clone(), value);
            }
        }
        (!result.is_empty()).then_some(Value::Mapping(result))
    } else {
        (existing.is_none() && comments.get(path) != Some(defaults))
            .then(|| defaults.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_missing_fields_are_appended_once() {
        let defaults = serde_yaml_ng::from_str(
            "font: {size: 19, hinting: false}\nwindow: {padding: {x: 2, y: 2}}",
        )
        .unwrap();
        let text = "# My settings\nfont: {hinting: true}\nwindow: {padding: {x: 0, y: 2}}";
        let addition = append_missing(text, &defaults).unwrap();
        assert_eq!(addition, "\n# font:\n#   size: 19\n");
        let updated = format!("{text}{addition}");
        assert!(append_missing(&updated, &defaults).unwrap().is_empty());
        let config = crate::config::Config::parse(&updated).unwrap();
        assert!(config.font.hinting);
        assert_eq!(config.window.padding.x, 0);
    }

    #[test]
    fn plain_comments_keep_their_parent_context() {
        let defaults = serde_yaml_ng::from_str("font: {size: 19}").unwrap();
        for text in [
            "# font:\n#   size: 19\n",
            "font:\n  hinting: false\n  # size: 19\n",
        ] {
            assert!(append_missing(text, &defaults).unwrap().is_empty());
        }
        assert!(
            !append_missing("# other:\n#   size: 19\n", &defaults)
                .unwrap()
                .is_empty()
        );
        assert!(
            append_missing("font: {size: 24}\n", &defaults)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn documented_defaults_match_runtime_defaults() {
        let documented = crate::config::Config::parse(include_str!(
            "../default-config.yaml"
        ))
        .unwrap();
        let runtime = crate::config::Config::default();
        assert_eq!(
            documented.font.size.pixels(1.0),
            runtime.font.size.pixels(1.0)
        );
        assert_eq!(documented.font.hinting, runtime.font.hinting);
        assert_eq!(documented.font.family, runtime.font.family);
        assert_eq!(
            documented.window.padding.physical(1.0),
            runtime.window.padding.physical(1.0)
        );
        assert_eq!(documented.colors.theme(), runtime.colors.theme());
    }
}
