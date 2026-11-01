use crate::{Error, Result};

const MAX_TEXT: usize = 20_000;
const MAX_NAME: usize = 64;
const MAX_QUERY: usize = 200;

pub fn message_text(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(Error::EmptyText);
    }
    if text.chars().count() > MAX_TEXT {
        return Err(Error::TextTooLong);
    }
    Ok(())
}

/// Returns the name with surrounding whitespace removed.
pub fn channel_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    let count = trimmed.chars().count();
    if !(1..=MAX_NAME).contains(&count) || trimmed.chars().any(char::is_control) {
        return Err(Error::InvalidArgument {
            message: "channel name must be 1-64 characters without control characters".into(),
        });
    }
    Ok(trimmed.to_owned())
}

pub fn search_query(query: &str) -> Result<()> {
    if query.trim().is_empty() || query.chars().count() > MAX_QUERY {
        return Err(Error::InvalidQuery);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_limits() {
        assert_eq!(message_text(" \n "), Err(Error::EmptyText));
        assert_eq!(message_text(&"あ".repeat(20_001)), Err(Error::TextTooLong));
        assert_eq!(message_text(&"あ".repeat(20_000)), Ok(()));
    }

    #[test]
    fn names_are_trimmed_and_checked() {
        assert_eq!(channel_name("  ログイン修正 ").unwrap(), "ログイン修正");
        assert!(channel_name("   ").is_err());
        assert!(channel_name("a\nb").is_err());
        assert!(channel_name(&"a".repeat(65)).is_err());
        assert!(channel_name(&"あ".repeat(64)).is_ok());
    }

    #[test]
    fn query_limits() {
        assert_eq!(search_query("  "), Err(Error::InvalidQuery));
        assert_eq!(search_query(&"a".repeat(201)), Err(Error::InvalidQuery));
        assert_eq!(search_query("あ"), Ok(()));
    }
}
