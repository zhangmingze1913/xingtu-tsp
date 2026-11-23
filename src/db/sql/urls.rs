//! Database URL normalization for Windows paths and MariaDB aliases.

pub(crate) fn sqlite_url_from_path(path: &str) -> String {
    // sqlx parses the URL with the `url` crate; Windows separators are not
    // valid in a URL, while SQLite accepts forward slashes on Windows too.
    let normalized = path.replace('\\', "/");
    // Absolute Windows paths need the `sqlite:///C:/...` form; relative paths
    // use `sqlite://data.db`.
    if normalized.len() >= 3 && normalized.as_bytes()[1] == b':' {
        format!("sqlite:///{normalized}?mode=rwc")
    } else {
        format!("sqlite://{normalized}?mode=rwc")
    }
}

/// Normalize a MariaDB URL to the `mysql://` scheme understood by sqlx.
pub(crate) fn mysql_url_from(url: &str) -> String {
    match url.strip_prefix("mariadb://") {
        Some(rest) => format!("mysql://{rest}"),
        None => url.to_string(),
    }
}
