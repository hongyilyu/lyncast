//! The Exec tokenizer. It reads an `Exec` value after key-file unescaping and gives the argv
//! that step 8 runs.

use std::mem;

/// What the field codes expand to. A launcher passes no files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecContext<'a> {
    /// `%c`: the app's translated Name.
    pub name: &'a str,
    /// `%i`: `--icon` and the Icon, or nothing without one.
    pub icon: Option<&'a str>,
    /// `%k`: the desktop file's path.
    pub path: &'a str,
}

/// Why an Exec value has no argv. An entry with one does not show, and an action with one is
/// skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecError {
    /// A quote that never closes, or a value whose expansion ends with the `#` of a comment,
    /// which GLib reports as the same error.
    UnterminatedQuote,
    /// A backslash ends the value outside quotes, with nothing to escape.
    TrailingBackslash,
    /// A `%` before a character that is not a field code, such as `%x`.
    UnknownCode(char),
    /// A `%` at the end of the value.
    LonePercent,
    /// `%i` inside a longer argument. It expands to two arguments, so it must stand alone.
    EmbeddedIcon,
    /// No argument is left once the field codes are gone.
    Empty,
}

/// Splits an Exec value into arguments and expands its field codes. It splits as
/// `g_shell_parse_argv` does once the codes expand.
///
/// Arguments split on unquoted spaces, tabs, and newlines. Double quotes group text, and inside
/// them a backslash escapes `"`, `` ` ``, `$`, `\`, and a newline, and stays before any other
/// character. Single quotes group text with no escapes. Outside quotes a backslash escapes the
/// next character, a backslash before a newline joins the lines, and a `#` that starts the value
/// or follows a space or a newline starts a comment that runs to the next newline. Field codes
/// expand inside and outside quotes, and an argument made only of codes that expand to nothing
/// disappears, while `""` stays an empty argument.
///
/// A code's value stays whole and unquoted. GLib differs in these known cases. It expands a code
/// before a backslash can escape its `%`. It shell-quotes `%c`, `%k`, and the Icon before it
/// splits, so inside double quotes the quotes stay, and inside single quotes a value with
/// whitespace splits and one with a quote fails to launch.
///
/// `sh -c 'echo a b'` gives `["sh", "-c", "echo a b"]`. With Name `Web Browser`,
/// `sh -c 'exec app --name=%c' x` gives `["sh", "-c", "exec app --name=Web Browser", "x"]`,
/// where GLib gives `["sh", "-c", "exec app --name=Web", "Browser", "x"]`.
pub fn tokenize(exec: &str, context: &ExecContext<'_>) -> Result<Vec<String>, ExecError> {
    let mut argv = Vec::new();
    let mut arg = String::new();
    // The argument holds text or quotes, so it stays even when it is empty.
    let mut literal = false;
    let mut quote = None;
    // GLib reads the character before a `#` in the expanded line, so a code that expands to
    // nothing leaves this as it was.
    let mut comment_may_start = true;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        let mut after = matches!(c, ' ' | '\n');
        match (quote, c) {
            (None, '#') if comment_may_start => {
                let mut comment = String::new();
                let mut closed = false;
                // The comment's newline neither splits nor joins, as in GLib. A `%` takes the
                // character after it as its code, as GLib expands codes before it splits, so
                // `%` before a newline expands to nothing and the comment runs on.
                while let Some(skipped) = chars.next() {
                    if skipped == '\n' {
                        closed = true;
                        break;
                    }
                    comment.push(skipped);
                    if skipped == '%' {
                        comment.extend(chars.next());
                    }
                }
                if !closed && expands_to_nothing(&comment, context.icon.is_some()) {
                    return Err(ExecError::UnterminatedQuote);
                }
                after = true;
            }
            (None, ' ' | '\t' | '\n') => {
                if mem::take(&mut literal) {
                    argv.push(mem::take(&mut arg));
                }
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                literal = true;
            }
            (Some(open), _) if c == open => quote = None,
            (None, '\\') => match chars.next() {
                None => return Err(ExecError::TrailingBackslash),
                Some('\n') => after = true,
                Some(escaped) => {
                    arg.push(escaped);
                    literal = true;
                    after = escaped == ' ';
                }
            },
            (Some('"'), '\\') => {
                match chars.next_if(|next| matches!(next, '"' | '`' | '$' | '\\' | '\n')) {
                    Some(escaped) => arg.push(escaped),
                    None => arg.push('\\'),
                }
            }
            (_, '%') => match chars.next() {
                None => return Err(ExecError::LonePercent),
                Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {
                    after = comment_may_start;
                }
                Some('%') => {
                    arg.push('%');
                    literal = true;
                }
                Some('c') => {
                    arg.push_str(context.name);
                    literal = true;
                }
                Some('k') => {
                    arg.push_str(context.path);
                    literal = true;
                }
                Some('i') => {
                    let alone = quote.is_none()
                        && !literal
                        && matches!(chars.peek(), None | Some(' ' | '\t' | '\n'));
                    if !alone {
                        return Err(ExecError::EmbeddedIcon);
                    }
                    match context.icon {
                        Some(icon) => argv.extend(["--icon".to_owned(), icon.to_owned()]),
                        None => after = comment_may_start,
                    }
                }
                Some(code) => return Err(ExecError::UnknownCode(code)),
            },
            (_, c) => {
                arg.push(c);
                literal = true;
            }
        }
        comment_may_start = after;
    }
    if quote.is_some() {
        return Err(ExecError::UnterminatedQuote);
    }
    if literal {
        argv.push(arg);
    }
    if argv.is_empty() {
        return Err(ExecError::Empty);
    }
    Ok(argv)
}

/// GLib expands field codes before it splits, and fails a line that ends with a comment's `#`.
/// A last comment that holds only codes GLib expands to nothing leaves its `#` at the end.
fn expands_to_nothing(comment: &str, icon: bool) -> bool {
    let mut chars = comment.chars();
    while let Some(c) = chars.next() {
        match (c, chars.next()) {
            ('%', Some('c' | 'k' | '%')) => return false,
            ('%', Some('i')) if icon => return false,
            ('%', Some(_)) => {}
            _ => return false,
        }
    }
    true
}
