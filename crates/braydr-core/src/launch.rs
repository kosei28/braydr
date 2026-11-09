//! Builds the shell line for `terminal.run` (`docs/product-spec.md` 8.1).

pub const DEFAULT_COMMAND: &str = "claude";
pub const DEFAULT_PROMPT: &str = "/braydr";
pub const ENV_CHANNEL_ID: &str = "BRAYDR_CHANNEL_ID";
/// The socket `braydr-mcp` connects to, so a session reaches the app that started it.
pub const ENV_IPC_SOCKET: &str = "BRAYDR_IPC_SOCKET";

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

/// Runs `line` in the user's own shell. An app opened from Finder has a minimal PATH, and so does
/// the core it starts, so tools installed through the shell's profile would not be found.
pub fn in_user_shell(line: &str) -> String {
    format!("exec \"${{SHELL:-/bin/sh}}\" -ilc {}", shell_quote(line))
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

    #[test]
    fn user_shell_finds_what_its_profile_adds() {
        let home = std::env::temp_dir().join(format!("braydr-shell-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshrc"), "export PATH=\"/from/profile:$PATH\"\n").unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &in_user_shell("printf '%s' \"$PATH\"")])
            .env_clear()
            .env("HOME", &home)
            .env("ZDOTDIR", &home)
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/zsh")
            .output()
            .unwrap();
        let path = String::from_utf8_lossy(&output.stdout);
        assert!(path.contains("/from/profile"), "PATH was {path}");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
