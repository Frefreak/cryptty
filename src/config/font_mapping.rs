use std::ops::RangeInclusive;

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "RawMapping")]
pub struct FontMapping {
    ranges: Vec<CodepointRange>,
    family: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMapping {
    ranges: Vec<CodepointRange>,
    family: String,
}

impl TryFrom<RawMapping> for FontMapping {
    type Error = &'static str;

    fn try_from(raw: RawMapping) -> Result<Self, Self::Error> {
        if raw.ranges.is_empty() || raw.family.trim().is_empty() {
            return Err("Font mappings require ranges and a font family");
        }
        Ok(Self {
            ranges: raw.ranges,
            family: raw.family.trim().to_owned(),
        })
    }
}

impl FontMapping {
    pub(super) fn defaults() -> Vec<Self> {
        let mut mappings = vec![Self::cjk_default()];
        if cfg!(target_os = "macos") {
            mappings.push(
                serde_yaml_ng::from_str(include_str!(
                    "../default-font-macos.yaml"
                ))
                .expect("Valid built-in macOS font mapping"),
            );
        }
        mappings
    }

    fn cjk_default() -> Self {
        Self {
            ranges: [
                '\u{2e80}'..='\u{9fff}',
                '\u{f900}'..='\u{faff}',
                '\u{fe10}'..='\u{fe1f}',
                '\u{fe30}'..='\u{fe4f}',
                '\u{ff00}'..='\u{ffef}',
                '\u{20000}'..='\u{3ffff}',
            ]
            .into_iter()
            .map(CodepointRange)
            .collect(),
            family: "LXGW WenKai Mono Screen".to_string(),
        }
    }

    pub fn contains(&self, ch: char) -> bool {
        self.ranges.iter().any(|range| range.0.contains(&ch))
    }

    pub fn family(&self) -> &str {
        &self.family
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
struct CodepointRange(RangeInclusive<char>);

impl TryFrom<String> for CodepointRange {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let (start, end) = text
            .split_once('-')
            .unwrap_or((text.as_str(), text.as_str()));
        let start = codepoint(start)?;
        let end = codepoint(end)?;
        if start > end {
            return Err("Font range start must precede its end");
        }
        Ok(Self(start..=end))
    }
}

fn codepoint(text: &str) -> Result<char, &'static str> {
    let invalid = "Expected a Unicode scalar such as U+23CE";
    let hex = text.trim().strip_prefix("U+").ok_or(invalid)?;
    if hex.is_empty()
        || hex.len() > 6
        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid);
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or(invalid)
}
