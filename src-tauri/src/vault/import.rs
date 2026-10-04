use super::ImportItem;

// ─── ENV file parser ──────────────────────────────────────────────────────────

/// Parses a .env file with the same grammar crypt-env writes
/// (`envfile::serialize_value`): bare, single- and double-quoted values,
/// comments and blank lines skipped. Each pair becomes a "secret" type item.
pub fn parse_env_file(content: &str) -> Vec<ImportItem> {
    crate::envfile::parse_dotenv(content)
        .into_iter()
        .map(|(key, val)| ImportItem {
            name: key,
            value: Some(val),
            username: None,
            password: None,
            url: None,
            notes: None,
            item_type: "secret".to_string(),
        })
        .collect()
}

// ─── CSV helpers ──────────────────────────────────────────────────────────────

/// Split a single CSV line into fields, respecting double-quoted fields.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    // A second consecutive quote inside a quoted field is an escaped quote.
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        current.push('"');
                    } else {
                        in_quotes = false;
                    }
                } else {
                    in_quotes = true;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current.trim().to_string());
    fields
}

/// Return the index of a header column by trying several case-insensitive names.
fn find_col(headers: &[String], candidates: &[&str]) -> Option<usize> {
    candidates.iter().find_map(|c| {
        headers.iter().position(|h| h.to_lowercase() == c.to_lowercase())
    })
}

fn opt_field(row: &[String], idx: Option<usize>) -> Option<String> {
    idx.and_then(|i| row.get(i))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ─── Bitwarden CSV parser ─────────────────────────────────────────────────────

/// Parses a Bitwarden CSV export.
/// Expected header (at minimum): folder,favorite,type,name,notes,fields,reprompt,
///                                login_uri,login_username,login_password,login_totp
/// "login" entries become "credential", everything else becomes "note".
pub fn parse_bitwarden_csv(content: &str) -> Vec<ImportItem> {
    let mut lines = content.lines();
    let header_line = match lines.next() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let headers = split_csv_line(header_line);

    let col_type     = find_col(&headers, &["type"]);
    let col_name     = find_col(&headers, &["name"]);
    let col_notes    = find_col(&headers, &["notes"]);
    let col_uri      = find_col(&headers, &["login_uri"]);
    let col_username = find_col(&headers, &["login_username"]);
    let col_password = find_col(&headers, &["login_password"]);

    lines
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let row = split_csv_line(line);
            let name = opt_field(&row, col_name).unwrap_or_else(|| "Unnamed".to_string());
            let entry_type = opt_field(&row, col_type)
                .unwrap_or_default()
                .to_lowercase();

            if entry_type == "login" {
                Some(ImportItem {
                    name,
                    value: None,
                    username: opt_field(&row, col_username),
                    password: opt_field(&row, col_password),
                    url: opt_field(&row, col_uri),
                    notes: opt_field(&row, col_notes),
                    item_type: "credential".to_string(),
                })
            } else {
                // Secure notes and other types → note
                let notes = opt_field(&row, col_notes);
                if name == "Unnamed" && notes.is_none() {
                    return None;
                }
                Some(ImportItem {
                    name,
                    value: None,
                    username: None,
                    password: None,
                    url: None,
                    notes,
                    item_type: "note".to_string(),
                })
            }
        })
        .collect()
}

// ─── 1Password CSV parser ─────────────────────────────────────────────────────

/// Parses a 1Password CSV export.
/// Common columns: Title,Username,Password,URL,Notes (and variants).
/// All entries become "credential".
pub fn parse_1password_csv(content: &str) -> Vec<ImportItem> {
    let mut lines = content.lines();
    let header_line = match lines.next() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let headers = split_csv_line(header_line);

    let col_name     = find_col(&headers, &["title", "name"]);
    let col_username = find_col(&headers, &["username", "user", "login"]);
    let col_password = find_col(&headers, &["password", "pass"]);
    let col_url      = find_col(&headers, &["url", "website", "login_url"]);
    let col_notes    = find_col(&headers, &["notes", "note", "comments"]);

    lines
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let row = split_csv_line(line);
            let name = opt_field(&row, col_name).unwrap_or_else(|| "Unnamed".to_string());
            Some(ImportItem {
                name,
                value: None,
                username: opt_field(&row, col_username),
                password: opt_field(&row, col_password),
                url: opt_field(&row, col_url),
                notes: opt_field(&row, col_notes),
                item_type: "credential".to_string(),
            })
        })
        .collect()
}

// ─── Generic CSV parser ───────────────────────────────────────────────────────

/// Parses a generic CSV file using header detection.
/// If a "password" column is found, items become "credential".
/// Otherwise the first column is treated as name and the second as value → "secret".
pub fn parse_csv_generic(content: &str) -> Vec<ImportItem> {
    let mut lines = content.lines();
    let header_line = match lines.next() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let headers = split_csv_line(header_line);

    let col_name     = find_col(&headers, &["name", "title", "key", "label"]);
    let col_value    = find_col(&headers, &["value", "secret", "token", "data"]);
    let col_username = find_col(&headers, &["username", "user", "login", "email"]);
    let col_password = find_col(&headers, &["password", "pass", "pwd"]);
    let col_url      = find_col(&headers, &["url", "website", "uri"]);
    let col_notes    = find_col(&headers, &["notes", "note", "comment", "comments"]);

    let is_credential = col_password.is_some();

    lines
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let row = split_csv_line(line);
            if row.is_empty() {
                return None;
            }

            // Fall back to positional columns when header names didn't match
            let name = opt_field(&row, col_name)
                .or_else(|| row.first().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "Unnamed".to_string());

            if is_credential {
                Some(ImportItem {
                    name,
                    value: None,
                    username: opt_field(&row, col_username),
                    password: opt_field(&row, col_password),
                    url: opt_field(&row, col_url),
                    notes: opt_field(&row, col_notes),
                    item_type: "credential".to_string(),
                })
            } else {
                let value = opt_field(&row, col_value)
                    .or_else(|| row.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
                Some(ImportItem {
                    name,
                    value,
                    username: None,
                    password: None,
                    url: None,
                    notes: opt_field(&row, col_notes),
                    item_type: "secret".to_string(),
                })
            }
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_file() {
        let content = "# comment\nAPI_KEY=abc123\nDB_URL=postgres://localhost\n\n";
        let items = parse_env_file(content);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "API_KEY");
        assert_eq!(items[0].value.as_deref(), Some("abc123"));
        assert_eq!(items[0].item_type, "secret");
    }

    #[test]
    fn test_parse_bitwarden_csv_login() {
        let csv = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
                   ,0,login,GitHub,,,,https://github.com,user@example.com,s3cr3t,";
        let items = parse_bitwarden_csv(csv);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_type, "credential");
        assert_eq!(items[0].name, "GitHub");
        assert_eq!(items[0].username.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn test_parse_1password_csv() {
        let csv = "Title,Username,Password,URL,Notes\nGitHub,alice,hunter2,https://github.com,";
        let items = parse_1password_csv(csv);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "GitHub");
        assert_eq!(items[0].item_type, "credential");
    }

    #[test]
    fn test_parse_csv_generic_secret() {
        let csv = "name,value\nMY_TOKEN,tok_live_abc";
        let items = parse_csv_generic(csv);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_type, "secret");
        assert_eq!(items[0].value.as_deref(), Some("tok_live_abc"));
    }

    #[test]
    fn test_split_csv_line_quoted() {
        let row = split_csv_line(r#"hello,"world, comma","quote ""test"""#);
        assert_eq!(row[0], "hello");
        assert_eq!(row[1], "world, comma");
        assert_eq!(row[2], r#"quote "test""#);
    }
}
