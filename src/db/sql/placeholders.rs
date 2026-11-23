//! `?N` placeholder translation and DDL statement splitting.

/// Rewrite `?` / `?N` placeholders into PostgreSQL's `$N` form.
pub(crate) fn rewrite_placeholders(query: &str) -> String {
    let mut output = String::with_capacity(query.len() + 8);
    let mut chars = query.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut next_plain = 1usize;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote => {
                output.push(ch);
                if in_single_quote {
                    // SQL escapes a quote inside a string by doubling it.
                    if chars.peek() == Some(&'\'') {
                        if let Some(escaped) = chars.next() {
                            output.push(escaped);
                        }
                    } else {
                        in_single_quote = false;
                    }
                } else {
                    in_single_quote = true;
                }
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                output.push(ch);
            }
            '?' if !in_single_quote && !in_double_quote => {
                let mut digits = String::new();
                while let Some(digit) = chars.peek().copied().filter(|c| c.is_ascii_digit()) {
                    digits.push(digit);
                    chars.next();
                }
                output.push('$');
                if digits.is_empty() {
                    output.push_str(&next_plain.to_string());
                    next_plain += 1;
                } else {
                    output.push_str(&digits);
                    if let Ok(index) = digits.parse::<usize>() {
                        next_plain = next_plain.max(index + 1);
                    }
                }
            }
            _ => output.push(ch),
        }
    }
    output
}

/// Rewrite canonical `?N` placeholders into MySQL's unnamed `?` markers.
///
/// Returns the rewritten SQL plus the zero-based bound-parameter number for
/// every marker, in marker order. Repeated markers therefore repeat their
/// parameter number, which lets the caller duplicate the bound value.
pub(crate) fn mysql_placeholders(query: &str) -> (String, Vec<usize>) {
    let mut output = String::with_capacity(query.len());
    let mut occurrences = Vec::new();
    let mut chars = query.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut next_plain = 1usize;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote => {
                output.push(ch);
                if in_single_quote {
                    if chars.peek() == Some(&'\'') {
                        if let Some(escaped) = chars.next() {
                            output.push(escaped);
                        }
                    } else {
                        in_single_quote = false;
                    }
                } else {
                    in_single_quote = true;
                }
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                output.push(ch);
            }
            '?' if !in_single_quote && !in_double_quote => {
                let mut digits = String::new();
                while let Some(digit) = chars.peek().copied().filter(|c| c.is_ascii_digit()) {
                    digits.push(digit);
                    chars.next();
                }
                output.push('?');
                if digits.is_empty() {
                    occurrences.push(next_plain - 1);
                    next_plain += 1;
                } else {
                    let index = digits.parse::<usize>().unwrap_or(1);
                    occurrences.push(index.saturating_sub(1));
                    next_plain = next_plain.max(index + 1);
                }
            }
            _ => output.push(ch),
        }
    }
    (output, occurrences)
}

/// Split a SQL script into individual statements for backends that cannot run
/// multi-statement prepared SQL. Semicolons inside quoted strings are kept.
pub(crate) fn split_sql_statements(script: &str) -> Vec<&str> {
    let bytes = script.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' if !in_double_quote => {
                if in_single_quote && bytes.get(index + 1) == Some(&b'\'') {
                    index += 1;
                } else {
                    in_single_quote = !in_single_quote;
                }
            }
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b';' if !in_single_quote && !in_double_quote => {
                let statement = script[start..index].trim();
                if !statement.is_empty() {
                    statements.push(statement);
                }
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }

    let tail = script[start..].trim();
    if !tail.is_empty() {
        statements.push(tail);
    }
    statements
}
