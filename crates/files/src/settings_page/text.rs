//! How lists and arguments are typed into one line of text.

/// Splits `line` at spaces; double or single quotes keep spaces in an
/// argument. No other shell syntax is interpreted.
pub(crate) fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    for character in line.chars() {
        match (quote, character) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(character);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// The line that [`split_args`] reads back as `args`.
pub(crate) fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.contains(char::is_whitespace) || arg.contains(['"', '\'']) {
                if arg.contains('"') {
                    format!("'{arg}'")
                } else {
                    format!("\"{arg}\"")
                }
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits a comma-separated list, dropping empty items.
pub(crate) fn split_list(line: &str) -> Vec<String> {
    line.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_round_trip_with_quotes() {
        let args = split_args(r#"-R +{file} "two words" 'say "hi"' """#);
        assert_eq!(args, ["-R", "+{file}", "two words", "say \"hi\"", ""]);
        assert_eq!(split_args(&join_args(&args)), args);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn lists_are_comma_separated() {
        assert_eq!(split_list(" md, txt ,,rs"), ["md", "txt", "rs"]);
    }
}
