use adscan_model::{LogEntry, LogSearchHit};
use async_trait::async_trait;
use rusqlite::{Row, params};
use serde_json::Value;

use super::{SqliteStore, enum_from_text, enum_text, from_json, to_json};
use crate::application::ports::{LogRepository, RepoResult};

/// Private-use characters that cannot occur in the HTML-escaped text; SQLite
/// wraps matches in them and they become `<mark>` tags after escaping.
const MATCH_START: char = '\u{E000}';
const MATCH_END: char = '\u{E001}';
const SNIPPET_TOKENS: u32 = 24;

const LOG_COLUMNS: &str = "l.id, l.scan_id, l.step_id, l.node, l.ts, l.level, l.kind, l.message, l.data";

#[async_trait]
impl LogRepository for SqliteStore {
    async fn append(&self, entry: &LogEntry) -> RepoResult<()> {
        let entry = entry.clone();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO logs (id, scan_id, step_id, node, ts, level, kind, message, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    entry.id,
                    entry.scan_id,
                    entry.step_id,
                    enum_text(&entry.node)?,
                    entry.ts,
                    enum_text(&entry.level)?,
                    enum_text(&entry.kind)?,
                    entry.message,
                    entry.data.as_ref().map(to_json).transpose()?,
                ],
            )?;
            transaction.execute(
                "INSERT INTO logs_fts (rowid, message, data_text) VALUES (?1, ?2, ?3)",
                params![
                    transaction.last_insert_rowid(),
                    entry.message,
                    entry.data.as_ref().map(text_projection).unwrap_or_default(),
                ],
            )?;
            transaction.commit()
        })
        .await
    }

    async fn list(&self, scan_id: &str, step_id: Option<&str>) -> RepoResult<Vec<LogEntry>> {
        let scan_id = scan_id.to_string();
        let step_id = step_id.map(str::to_string);
        self.call(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {LOG_COLUMNS} FROM logs l
                 WHERE l.scan_id = ?1 AND (?2 IS NULL OR l.step_id = ?2)
                 ORDER BY l.seq"
            ))?;
            statement
                .query_map(params![scan_id, step_id], read_entry)?
                .collect()
        })
        .await
    }

    async fn search(&self, query: &str, limit: u32) -> RepoResult<Vec<LogSearchHit>> {
        let Some(match_expression) = fts_query(query) else {
            return Ok(Vec::new());
        };
        self.call(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {LOG_COLUMNS}, s.url, snippet(logs_fts, -1, ?2, ?3, '…', ?4)
                 FROM logs_fts
                 JOIN logs l ON l.seq = logs_fts.rowid
                 JOIN scans s ON s.id = l.scan_id
                 WHERE logs_fts MATCH ?1
                 ORDER BY rank, l.seq DESC
                 LIMIT ?5"
            ))?;
            statement
                .query_map(
                    params![
                        match_expression,
                        MATCH_START.to_string(),
                        MATCH_END.to_string(),
                        SNIPPET_TOKENS,
                        limit
                    ],
                    |row| {
                        Ok(LogSearchHit {
                            entry: read_entry(row)?,
                            scan_url: row.get(9)?,
                            snippet: mark_matches(&row.get::<_, String>(10)?),
                        })
                    },
                )?
                .collect()
        })
        .await
    }
}

fn read_entry(row: &Row<'_>) -> rusqlite::Result<LogEntry> {
    Ok(LogEntry {
        id: row.get(0)?,
        scan_id: row.get(1)?,
        step_id: row.get(2)?,
        node: enum_from_text(row.get(3)?)?,
        ts: row.get(4)?,
        level: enum_from_text(row.get(5)?)?,
        kind: enum_from_text(row.get(6)?)?,
        message: row.get(7)?,
        data: row
            .get::<_, Option<String>>(8)?
            .map(|data| from_json(&data))
            .transpose()?,
    })
}

/// Turns free text into an FTS5 expression that cannot be a syntax error:
/// every alphanumeric run becomes a quoted term (implicit AND) and the last
/// one a prefix match, so results appear while the user is still typing.
/// `None` when the text has nothing searchable.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<&str> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect();
    let (last, complete) = terms.split_last()?;
    let mut expression: Vec<String> = complete.iter().map(|term| format!("\"{term}\"")).collect();
    expression.push(format!("\"{last}\"*"));
    Some(expression.join(" "))
}

/// Keys and scalar values of the structured payload as searchable text.
fn text_projection(data: &Value) -> String {
    fn collect(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Null => {}
            Value::Bool(flag) => out.push(flag.to_string()),
            Value::Number(number) => out.push(number.to_string()),
            Value::String(text) => out.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
            Value::Object(fields) => {
                for (key, value) in fields {
                    out.push(key.clone());
                    collect(value, out);
                }
            }
        }
    }
    let mut parts = Vec::new();
    collect(data, &mut parts);
    parts.join(" ")
}

fn mark_matches(snippet: &str) -> String {
    let mut html = String::with_capacity(snippet.len() + 16);
    for c in snippet.chars() {
        match c {
            MATCH_START => html.push_str("<mark>"),
            MATCH_END => html.push_str("</mark>"),
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            other => html.push(other),
        }
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fts_query_quotes_terms_and_prefixes_the_last() {
        assert_eq!(fts_query("device detect").as_deref(), Some("\"device\" \"detect\"*"));
        assert_eq!(fts_query("  isMobile( ").as_deref(), Some("\"isMobile\"*"));
        assert_eq!(fts_query("a-b").as_deref(), Some("\"a\" \"b\"*"));
        assert_eq!(fts_query("\"OR\" NEAR(").as_deref(), Some("\"OR\" \"NEAR\"*"));
        assert_eq!(fts_query("\"*:^()-"), None);
        assert_eq!(fts_query(""), None);
    }

    #[test]
    fn projection_flattens_keys_and_values() {
        let text = text_projection(&json!({ "tool": "read_script", "input": { "start_line": 10, "ok": true } }));
        assert_eq!(text, "input ok true start_line 10 tool read_script");
    }

    #[test]
    fn snippet_is_escaped_and_only_matches_are_marked() {
        let raw = format!("<script>alert('x')</script> & {MATCH_START}pbjs{MATCH_END}");
        assert_eq!(
            mark_matches(&raw),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; <mark>pbjs</mark>"
        );
    }
}
