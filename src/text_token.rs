//! Text boundaries shared by automatic links and copy markers.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Url,
    Path,
}

pub(crate) fn trim_url(text: &str) -> &str {
    trim_token(text, Kind::Url, "")
}

pub(crate) fn trim_path<'a>(text: &'a str, before: &str) -> &'a str {
    trim_token(text, Kind::Path, before)
}

fn trim_token<'a>(text: &'a str, kind: Kind, before: &str) -> &'a str {
    let closing = match before.chars().next_back() {
        Some('(') => Some(')'),
        Some('[') => Some(']'),
        Some('{') => Some('}'),
        Some('（') => Some('）'),
        _ => None,
    };
    let text = &text[..token_boundary(text, kind, closing)];
    let text = &text[..text.len() - trailing_punctuation(text)];
    if kind == Kind::Path {
        &text[..text.len() - location_suffix(text)]
    } else {
        text
    }
}

fn token_boundary(text: &str, kind: Kind, closing: Option<char>) -> usize {
    let mut depth = [0usize; 4];
    for (offset, ch) in text.char_indices() {
        if kind == Kind::Url
            && (ch.is_whitespace()
                || ch.is_control()
                || matches!(ch, '<' | '>' | '\"' | '\''))
        {
            return offset;
        }
        let (index, opening) = match ch {
            '(' => (0, true),
            '[' => (1, true),
            '{' => (2, true),
            ')' => (0, false),
            ']' => (1, false),
            '}' => (2, false),
            '（' => (3, true),
            '）' => (3, false),
            '，' | '；' | '。' | '！' | '？' | '、' if kind == Kind::Url =>
            {
                return offset;
            }
            _ => continue,
        };
        if opening {
            depth[index] += 1;
        } else if depth[index] == 0 {
            if kind == Kind::Url || closing == Some(ch) {
                return offset;
            }
        } else {
            depth[index] -= 1;
        }
    }
    text.len()
}

fn trailing_punctuation(text: &str) -> usize {
    let mut end = text.len();
    while let Some(ch) = text[..end].chars().next_back() {
        let unmatched = match ch {
            ')' => {
                text[..end].matches(')').count()
                    > text[..end].matches('(').count()
            }
            ']' => {
                text[..end].matches(']').count()
                    > text[..end].matches('[').count()
            }
            '}' => {
                text[..end].matches('}').count()
                    > text[..end].matches('{').count()
            }
            _ => false,
        };
        if !unmatched && !matches!(ch, ',' | '，' | ';' | '.' | ':' | '!' | '?')
        {
            break;
        }
        end -= ch.len_utf8();
    }
    text.len() - end
}

fn location_suffix(text: &str) -> usize {
    let mut end = text.len();
    for _ in 0..2 {
        let Some((prefix, suffix)) = text[..end].rsplit_once(':') else {
            break;
        };
        if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        end = prefix.len();
    }
    text.len() - end
}
