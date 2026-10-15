use std::collections::BTreeMap;

use anyhow::Result;
use serde_yaml_ng::{Mapping, Value};

pub(super) fn updated(text: &str) -> Result<String> {
    let defaults =
        serde_yaml_ng::from_str(include_str!("../default-config.yaml"))?;
    insert_missing(text, &defaults)
}

fn insert_missing(text: &str, defaults: &Value) -> Result<String> {
    let existing = serde_yaml_ng::from_str::<Value>(text)?;
    let comments = commented_fields(text);
    let Some(missing) = missing(defaults, Some(&existing), &[], &comments)
    else {
        return Ok(text.to_string());
    };
    let mut output = text.to_string();
    insert_comments(&mut output, &missing, &[], 0)?;
    Ok(output)
}

fn insert_comments(
    text: &mut String,
    missing: &Value,
    path: &[String],
    indent: usize,
) -> Result<()> {
    let fields = missing.as_mapping().expect("Missing defaults are a map");
    let mut remaining = Mapping::new();
    for (key, value) in fields {
        let mut child_path = path.to_vec();
        child_path.push(key.as_str().unwrap().to_string());
        if value.is_mapping()
            && let Some((_, child_indent)) = block_position(text, &child_path)
        {
            insert_comments(text, value, &child_path, child_indent)?;
        } else {
            remaining.insert(key.clone(), value.clone());
        }
    }
    if remaining.is_empty() {
        return Ok(());
    }
    let position =
        block_position(text, path).map_or(text.len(), |(position, _)| position);
    let mut comments = String::new();
    if position > 0 && !text[..position].ends_with('\n') {
        comments.push('\n');
    }
    for line in serde_yaml_ng::to_string(&remaining)?.lines() {
        comments.push_str(&" ".repeat(indent));
        comments.push_str("# ");
        comments.push_str(line);
        comments.push('\n');
    }
    text.insert_str(position, &comments);
    Ok(())
}

fn block_position(text: &str, target: &[String]) -> Option<(usize, usize)> {
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        while parents.last().is_some_and(|(level, _)| *level >= indent) {
            parents.pop();
        }
        let Ok(mapping) = serde_yaml_ng::from_str::<Mapping>(line) else {
            continue;
        };
        if mapping.len() != 1 {
            continue;
        }
        let (key, value) = mapping.into_iter().next().unwrap();
        let Some(key) = key.as_str() else { continue };
        if value.is_null() {
            parents.push((indent, key.to_string()));
            if parents.iter().map(|(_, key)| key).eq(target.iter()) {
                let child_indent = text[offset..]
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| line.len() - line.trim_start().len())
                    .filter(|child| *child > indent)
                    .unwrap_or(indent + 2);
                return Some((offset, child_indent));
            }
        }
    }
    None
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
        let addition = insert_missing(text, &defaults).unwrap();
        assert_eq!(addition, format!("{text}\n# font:\n#   size: 19\n"));
        let updated = addition;
        assert_eq!(insert_missing(&updated, &defaults).unwrap(), updated);
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
            assert_eq!(insert_missing(text, &defaults).unwrap(), text);
        }
        let unrelated = "# other:\n#   size: 19\n";
        assert_ne!(insert_missing(unrelated, &defaults).unwrap(), unrelated);
        let explicit = "font: {size: 24}\n";
        assert_eq!(insert_missing(explicit, &defaults).unwrap(), explicit);
    }

    #[test]
    fn missing_fields_are_inserted_in_existing_blocks() {
        let defaults = serde_yaml_ng::from_str(
            "font: {family: Example, size: 19, hinting: false}\n\
             window: {padding: {x: 2, y: 2}}",
        )
        .unwrap();
        let text =
            "# Mine\nfont:\n  hinting: true\nwindow:\n  padding:\n    x: 0\n";
        let updated = insert_missing(text, &defaults).unwrap();
        assert_eq!(
            updated,
            "# Mine\nfont:\n  # family: Example\n  # size: 19\n  hinting: true\nwindow:\n  padding:\n    # y: 2\n    x: 0\n"
        );
        assert_eq!(insert_missing(&updated, &defaults).unwrap(), updated);
        assert_eq!(
            serde_yaml_ng::from_str::<Value>(text).unwrap(),
            serde_yaml_ng::from_str::<Value>(&updated).unwrap()
        );
    }

    #[test]
    fn existing_indentation_and_missing_final_newline_are_preserved() {
        let defaults =
            serde_yaml_ng::from_str("font: {size: 19, hinting: false}")
                .unwrap();
        let text = "font:\n    hinting: true";
        assert_eq!(
            insert_missing(text, &defaults).unwrap(),
            "font:\n    # size: 19\n    hinting: true"
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
