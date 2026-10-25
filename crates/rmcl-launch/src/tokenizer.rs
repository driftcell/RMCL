use crate::LaunchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    Normal,
    Single,
    Double,
}

pub fn tokenize_legacy_arguments(input: &str) -> Result<Vec<String>, LaunchError> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut state = QuoteState::Normal;
    let mut characters = input.chars();

    while let Some(character) = characters.next() {
        match (state, character) {
            (QuoteState::Normal, '\'') => {
                state = QuoteState::Single;
                has_token = true;
            }
            (QuoteState::Single, '\'') => {
                state = QuoteState::Normal;
            }
            (QuoteState::Normal, '"') => {
                state = QuoteState::Double;
                has_token = true;
            }
            (QuoteState::Double, '"') => {
                state = QuoteState::Normal;
            }
            (QuoteState::Double, '`') => {
                let Some(escaped) = characters.next() else {
                    return Err(LaunchError::InvalidLegacyArguments(
                        "trailing escape character in double quotes".to_owned(),
                    ));
                };
                current.push(escaped);
                has_token = true;
            }
            (QuoteState::Normal, character) if character.is_whitespace() => {
                if has_token {
                    arguments.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            (_, character) => {
                current.push(character);
                has_token = true;
            }
        }
    }

    if state != QuoteState::Normal {
        return Err(LaunchError::InvalidLegacyArguments(
            "unterminated quoted argument".to_owned(),
        ));
    }
    if has_token {
        arguments.push(current);
    }
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_whitespace_and_preserves_quoted_content() {
        let arguments = tokenize_legacy_arguments("--name 'Steve Smith' --path \"a b\"").unwrap();
        assert_eq!(arguments, ["--name", "Steve Smith", "--path", "a b"]);
    }

    #[test]
    fn supports_empty_and_nested_quotes() {
        let arguments = tokenize_legacy_arguments("'' \"a 'b' c\" \"\" ").unwrap();
        assert_eq!(arguments, ["", "a 'b' c", ""]);
    }

    #[test]
    fn supports_backtick_escape_in_double_quotes() {
        let arguments = tokenize_legacy_arguments("\"a`\"b`\"c\"").unwrap();
        assert_eq!(arguments, ["a\"b\"c"]);
    }

    #[test]
    fn rejects_unclosed_quotes_and_trailing_escape() {
        assert!(tokenize_legacy_arguments("\"unterminated").is_err());
        assert!(tokenize_legacy_arguments("\"trailing`").is_err());
    }
}
