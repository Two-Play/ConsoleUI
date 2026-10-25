//! Minimal parser for Valve's KeyValues text format (`.vdf`, `.acf`).

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// Child entry of an object; Valve capitalizes keys inconsistently, hence case-insensitive.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(pairs) => pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Value::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Value)] {
        match self {
            Value::Obj(pairs) => pairs,
            Value::Str(_) => &[],
        }
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Open,
    Close,
}

/// Parses a whole document; the result is always a `Value::Obj`.
pub fn parse(input: &str) -> Result<Value, String> {
    let tokens = tokenize(input)?;
    let mut pos = 0;
    let root = parse_pairs(&tokens, &mut pos)?;
    match tokens.get(pos) {
        None => Ok(Value::Obj(root)),
        Some(_) => Err("Unexpected closing brace".into()),
    }
}

fn parse_pairs(tokens: &[Token], pos: &mut usize) -> Result<Vec<(String, Value)>, String> {
    let mut pairs = Vec::new();
    while let Some(Token::Text(key)) = tokens.get(*pos) {
        *pos += 1;
        let value = match tokens.get(*pos) {
            Some(Token::Text(text)) => {
                *pos += 1;
                Value::Str(text.clone())
            }
            Some(Token::Open) => {
                *pos += 1;
                let children = parse_pairs(tokens, pos)?;
                if tokens.get(*pos) != Some(&Token::Close) {
                    return Err(format!("Missing closing brace for \"{key}\""));
                }
                *pos += 1;
                Value::Obj(children)
            }
            _ => return Err(format!("No value for \"{key}\"")),
        };
        pairs.push((key.clone(), value));
    }
    Ok(pairs)
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut text = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => text.push('\n'),
                            Some('t') => text.push('\t'),
                            Some(other) => text.push(other),
                            None => return Err("Incomplete escape sequence".into()),
                        },
                        Some(other) => text.push(other),
                        None => return Err("Unterminated string".into()),
                    }
                }
                tokens.push(Token::Text(text));
            }
            other => {
                let mut text = String::from(other);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || next == '{' || next == '}' || next == '"' {
                        break;
                    }
                    text.push(next);
                    chars.next();
                }
                tokens.push(Token::Text(text));
            }
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_objects() {
        let doc = parse(
            r#"
            "libraryfolders"
            {
                "0" { "path" "/home/a/Steam" "apps" { "730" "123" } }
                // Kommentar
                "1" { "path" "/mnt/games" }
            }"#,
        )
        .unwrap();
        let folders = doc.get("libraryfolders").unwrap();
        assert_eq!(folders.entries().len(), 2);
        assert_eq!(folders.get("1").unwrap().get("path").unwrap().as_str(), Some("/mnt/games"));
        assert_eq!(folders.get("0").unwrap().get("apps").unwrap().get("730").unwrap().as_str(), Some("123"));
    }

    #[test]
    fn keys_are_case_insensitive_and_escapes_work() {
        let doc = parse(r#""AppState" { "Name" "Say \"Hi\"\\" }"#).unwrap();
        assert_eq!(doc.get("appstate").unwrap().get("name").unwrap().as_str(), Some(r#"Say "Hi"\"#));
    }

    #[test]
    fn rejects_broken_input() {
        assert!(parse(r#""a" { "b" "c""#).is_err());
        assert!(parse(r#""a" "unterminated"#).is_err());
        assert!(parse(r#""a" }"#).is_err());
    }
}
