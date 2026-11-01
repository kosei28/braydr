//! Builds the shell line for `terminal.run` (`docs/product-spec.md` 8.1).

pub const DEFAULT_COMMAND: &str = "claude";
pub const DEFAULT_PROMPT: &str = "/braydr";

/// Quotes `value` as one POSIX shell word.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The command runs as written; the prompt becomes its last argument.
pub fn shell_line(command: &str, prompt: &str) -> String {
    if prompt.is_empty() {
        return command.to_owned();
    }
    format!("{command} {}", shell_quote(prompt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_quoted_prompt() {
        assert_eq!(
            shell_line("claude --model opus", "/braydr"),
            "claude --model opus '/braydr'"
        );
    }

    #[test]
    fn empty_prompt_adds_nothing() {
        assert_eq!(shell_line("claude", ""), "claude");
    }

    #[test]
    fn quotes_survive_a_real_shell() {
        let prompt = "it's $HOME `x` \"y\"\n次の行";
        let line = shell_line("printf %s", prompt);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&line)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), prompt);
    }
}
