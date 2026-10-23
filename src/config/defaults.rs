use std::collections::BTreeMap;

use anyhow::Result;
use serde_yaml_ng::{Mapping, Value};

pub(super) fn updated(text: &str) -> Result<String> {
    insert_missing(text, &values()?)
}

fn values() -> Result<Value> {
    Ok(serde_yaml_ng::from_str(include_str!(
        "../default-config.yaml"
    ))?)
}

fn insert_missing(text: &str, defaults: &Value) -> Result<String> {
    let existing = serde_yaml_ng::from_str::<Value>(text)?;
    let comments = commented_fields(text);
    let Some(missing) = missing(defaults, Some(&existing), &[], &comments)
    else {
        return Ok(text.to_string());
    };
    let mut output = text.to_string();
    insert_comments(&mut output, &missing, &[], "# ")?;
    Ok(output)
}

fn insert_comments(
    text: &mut String,
    missing: &Value,
    path: &[String],
    prefix: &str,
) -> Result<()> {
    let fields = missing.as_mapping().expect("Missing defaults are a map");
    let mut remaining = Mapping::new();
    for (key, value) in fields {
        let mut child_path = path.to_vec();
        child_path.push(key.as_str().unwrap().to_string());
        if value.is_mapping()
            && let Some((_, child_prefix)) = block_position(text, &child_path)
        {
            insert_comments(text, value, &child_path, &child_prefix)?;
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
        comments.push_str(prefix);
        comments.push_str(line);
        comments.push('\n');
    }
    text.insert_str(position, &comments);
    Ok(())
}

fn yaml_line(line: &str) -> (&str, usize, Option<usize>) {
    let outer = line.len() - line.trim_start().len();
    match line.trim_start().strip_prefix("# ") {
        Some(yaml) => (
            yaml,
            outer + yaml.len() - yaml.trim_start().len(),
            Some(outer),
        ),
        None => (line, outer, None),
    }
}

fn block_position(text: &str, target: &[String]) -> Option<(usize, String)> {
    find_block(text, target, false).or_else(|| find_block(text, target, true))
}

fn find_block(
    text: &str,
    target: &[String],
    include_comments: bool,
) -> Option<(usize, String)> {
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        let (yaml, indent, comment_outer) = yaml_line(line);
        if comment_outer.is_some() && !include_comments {
            continue;
        }
        let Ok(mapping) = serde_yaml_ng::from_str::<Mapping>(yaml) else {
            continue;
        };
        if mapping.len() != 1 {
            continue;
        }
        while parents.last().is_some_and(|(level, _)| *level >= indent) {
            parents.pop();
        }
        let (key, value) = mapping.into_iter().next().unwrap();
        let Some(key) = key.as_str() else { continue };
        if value.is_null() {
            parents.push((indent, key.to_string()));
            if parents.iter().map(|(_, key)| key).eq(target.iter()) {
                let child_indent = text[offset..]
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| yaml_line(line).1)
                    .filter(|child| *child > indent)
                    .unwrap_or(indent + 2);
                let prefix = match comment_outer {
                    Some(outer) => format!(
                        "{}# {}",
                        " ".repeat(outer),
                        " ".repeat(child_indent - outer)
                    ),
                    None => format!("{}# ", " ".repeat(child_indent)),
                };
                return Some((offset, prefix));
            }
        }
    }
    None
}

fn commented_fields(text: &str) -> BTreeMap<Vec<String>, Vec<Value>> {
    let mut fields: BTreeMap<Vec<String>, Vec<Value>> = BTreeMap::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut active_parents = Vec::new();
    let lines: Vec<_> = text.lines().collect();
    for (index, &line) in lines.iter().enumerate() {
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
        if !commented {
            parents.clone_from(&active_parents);
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
        if value.is_null() && commented {
            let mut block = yaml.to_string();
            for next in &lines[index + 1..] {
                let Some(child) = next.trim_start().strip_prefix("# ") else {
                    break;
                };
                let child_indent = next.len() - next.trim_start().len()
                    + child.len()
                    - child.trim_start().len();
                if child_indent < indent
                    || (child_indent == indent
                        && !child.trim_start().starts_with("- "))
                {
                    break;
                }
                block.push('\n');
                block.push_str(child);
            }
            if let Ok(parsed) = serde_yaml_ng::from_str::<Mapping>(&block)
                && let Some(value @ Value::Sequence(_)) = parsed.get(key)
            {
                let mut path: Vec<_> =
                    parents.iter().map(|(_, k)| k.clone()).collect();
                path.push(key.to_string());
                fields.entry(path).or_default().push(value.clone());
            }
        }
        if value.is_null() {
            parents.push((indent, key.to_string()));
        } else if commented {
            let mut path: Vec<_> =
                parents.iter().map(|(_, k)| k.clone()).collect();
            path.push(key.to_string());
            fields.entry(path).or_default().push(value);
        }
        if !commented {
            active_parents.clone_from(&parents);
        }
    }
    fields
}

fn missing(
    defaults: &Value,
    existing: Option<&Value>,
    path: &[String],
    comments: &BTreeMap<Vec<String>, Vec<Value>>,
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
        (existing.is_none()
            && !comments
                .get(path)
                .is_some_and(|values| values.contains(defaults)))
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
    fn prose_comments_do_not_hide_active_nested_blocks() {
        let defaults =
            serde_yaml_ng::from_str("window: {padding: {x: 2, y: 2}}").unwrap();
        let text = "window:\n# Note:\n  padding:\n    x: 0\n";
        let updated = insert_missing(text, &defaults).unwrap();
        assert_eq!(
            updated,
            "window:\n# Note:\n  padding:\n    # y: 2\n    x: 0\n"
        );
        assert_eq!(insert_missing(&updated, &defaults).unwrap(), updated);
    }

    #[test]
    fn new_defaults_extend_commented_parents_without_duplicate_sections() {
        let defaults = serde_yaml_ng::from_str(
            "window: {option_as_alt: only_left, padding: {x: 2, y: 2}}\n\
             selection: {word_characters: '/\\._-~:@%+=,#?'}",
        )
        .unwrap();
        let text = concat!(
            "# window:\n#   option_as_alt: only_left\n",
            "# selection:\n#   word_characters: /\\._-~:@%+=,#?\n",
        );
        let updated = insert_missing(text, &defaults).unwrap();
        assert_eq!(
            updated,
            concat!(
                "# window:\n#   padding:\n#     x: 2\n#     y: 2\n",
                "#   option_as_alt: only_left\n",
                "# selection:\n#   word_characters: /\\._-~:@%+=,#?\n",
            )
        );
        assert_eq!(insert_missing(&updated, &defaults).unwrap(), updated);
    }

    #[test]
    fn nested_commented_blocks_keep_their_comment_indentation() {
        let defaults =
            serde_yaml_ng::from_str("window: {padding: {x: 2, y: 2}}").unwrap();
        for (text, expected) in [
            (
                "# window:\n#   padding:\n#     x: 2\n",
                "# window:\n#   padding:\n#     y: 2\n#     x: 2\n",
            ),
            (
                "window:\n  # padding:\n  #   x: 2\n  option_as_alt: none\n",
                "window:\n  # padding:\n  #   y: 2\n  #   x: 2\n  option_as_alt: none\n",
            ),
        ] {
            let updated = insert_missing(text, &defaults).unwrap();
            assert_eq!(updated, expected);
            assert_eq!(insert_missing(&updated, &defaults).unwrap(), updated);
            assert_eq!(
                serde_yaml_ng::from_str::<Value>(text).unwrap(),
                serde_yaml_ng::from_str::<Value>(&updated).unwrap()
            );
        }
    }

    #[test]
    fn documented_defaults_match_runtime_defaults() {
        let documented: crate::config::Config =
            serde_yaml_ng::from_value(values().unwrap()).unwrap();
        let runtime = crate::config::Config::default();
        assert_eq!(
            documented.font.size.pixels(1.0),
            runtime.font.size.pixels(1.0)
        );
        assert_eq!(documented.font.hinting, runtime.font.hinting);
        assert_eq!(documented.font.family, runtime.font.family);
        assert_eq!(documented.font.fallback, runtime.font.fallback);
        assert_eq!(documented.font.codepoint_map, runtime.font.codepoint_map);
        assert_eq!(
            documented.window.padding.physical(1.0),
            runtime.window.padding.physical(1.0)
        );
        assert_eq!(documented.colors.theme(), runtime.colors.theme());
        assert_eq!(
            documented.selection.word_characters,
            runtime.selection.word_characters
        );
    }
}
