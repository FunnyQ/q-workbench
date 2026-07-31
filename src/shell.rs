/// Quotes one argument so a POSIX-compatible shell reads it literally.
pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Builds a shell command from literal arguments.
///
/// Shell syntax, such as the restart flow's `stty sane; printf '…';` prefix,
/// must remain separate. Only launcher paths and arguments belong here.
pub fn build_command(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{build_command, shell_quote};

    const CASES: &[&str] = &[
        "a b",
        "don't",
        r#"say "hello""#,
        "$HOME",
        "`date`",
        r"a\b",
        "*.rs",
        "file?",
        "~/project",
        "first\nsecond",
        "",
    ];

    #[test]
    fn shell_quote_quotes_every_character_class() {
        let expected = [
            "'a b'",
            r#"'don'\''t'"#,
            r#"'say "hello"'"#,
            "'$HOME'",
            "'`date`'",
            r"'a\b'",
            "'*.rs'",
            "'file?'",
            "'~/project'",
            "'first\nsecond'",
            "''",
        ];

        for (value, expected) in CASES.iter().zip(expected) {
            let quoted = shell_quote(value);
            assert_eq!(quoted, expected);
            assert!(quoted.starts_with('\''));
            assert!(quoted.ends_with('\''));
        }
    }

    #[test]
    fn shell_quote_round_trips_through_zsh() {
        for value in CASES {
            let script = format!("printf '%s' {}", shell_quote(value));
            let output = Command::new("zsh")
                .args(["-c", &script])
                .output()
                .expect("zsh must be available for shell quoting tests");

            assert!(output.status.success());
            assert_eq!(output.stdout, value.as_bytes());
            assert!(output.stderr.is_empty());
        }
    }

    #[test]
    fn build_command_quotes_and_joins_parts() {
        assert_eq!(
            build_command(&["hello".to_owned(), "world".to_owned()]),
            "'hello' 'world'"
        );
        assert_eq!(
            build_command(&["a b".to_owned(), "c".to_owned()]),
            "'a b' 'c'"
        );
        assert_eq!(build_command(&[]), "");
        assert_eq!(build_command(&["hello".to_owned()]), "'hello'");
    }
}
