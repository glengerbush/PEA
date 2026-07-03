//! Port of the Node SearchService (FTS5) — must produce byte-identical JSON.

use rusqlite::{types::Value as SqlValue, Connection};
use serde_json::{json, Map, Value};

const ALL_COLUMNS: [&str; 6] = ["subject", "body", "sender", "recipients", "attachments", "meta"];
// bm25 weights per FTS column order: (email_id, subject, body, sender, recipients, attachments, meta)
const BM25_WEIGHTS: &str = "0.0, 10.0, 2.0, 6.0, 3.0, 1.0, 2.0";

fn field_to_column(field: &str) -> Option<&'static str> {
    match field {
        "subject" => Some("subject"),
        "body" => Some("body"),
        "from" | "senderName" => Some("sender"),
        "to" | "cc" | "bcc" => Some("recipients"),
        "attachments.filename" | "attachments.content" => Some("attachments"),
        "userEmail" | "sourcePath" | "sourceLabels" | "tags" => Some("meta"),
        _ => None,
    }
}

const DEFAULT_FIELDS: [&str; 13] = [
    "subject",
    "body",
    "from",
    "senderName",
    "to",
    "cc",
    "bcc",
    "attachments.filename",
    "attachments.content",
    "userEmail",
    "sourcePath",
    "sourceLabels",
    "tags",
];

pub fn normalize_fields(raw: Option<&str>) -> Vec<String> {
    let fields: Vec<String> = raw
        .map(|value| {
            value
                .split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| DEFAULT_FIELDS.contains(&f.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if fields.is_empty() {
        DEFAULT_FIELDS.iter().map(|f| f.to_string()).collect()
    } else {
        fields
    }
}

fn sort_column(sort: Option<&str>) -> &'static str {
    match sort {
        Some("archivedAt") => "ae.archived_at",
        Some("sender") => "ae.sender_email",
        Some("subject") => "ae.subject",
        Some("sizeBytes") => "ae.size_bytes",
        _ => "ae.sent_at",
    }
}

fn clamp_positive(value: Option<&str>, fallback: i64, max: i64) -> i64 {
    let parsed = value.and_then(|v| v.parse::<f64>().ok());
    match parsed {
        Some(n) if n.is_finite() && n >= 1.0 => (n.floor() as i64).min(max),
        _ => fallback,
    }
}

/// Builds a safe FTS5 MATCH expression (quoted terms, trailing prefix `*`).
fn build_match(query: &str, fields: &[String], or_mode: bool) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.trim().is_empty())
        .take(12)
        .collect();
    if terms.is_empty() {
        return None;
    }
    let last = terms.len() - 1;
    let quoted: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == last {
                format!("\"{t}\"*")
            } else {
                format!("\"{t}\"")
            }
        })
        .collect();
    let body = quoted.join(if or_mode { " OR " } else { " " });

    let mut columns: Vec<&str> = fields
        .iter()
        .filter_map(|f| field_to_column(f))
        .collect();
    columns.dedup();
    let mut seen = Vec::new();
    columns.retain(|c| {
        if seen.contains(c) {
            false
        } else {
            seen.push(c);
            true
        }
    });
    if columns.len() >= ALL_COLUMNS.len() {
        Some(body)
    } else {
        Some(format!("{{{}}} : ({body})", columns.join(" ")))
    }
}

pub struct FilterSql {
    pub clause: String,
    pub params: Vec<SqlValue>,
}

fn to_timestamp(value: Option<&str>) -> Option<i64> {
    let value = value?;
    if let Ok(n) = value.parse::<i64>() {
        return Some(n);
    }
    // ISO date strings — parse with SQLite itself to avoid a chrono dependency here.
    None
}

/// Port of IngestionService.findGroupSourceIds — root = mergedIntoId ?? id,
/// plus all children of the root. `None` when the source id doesn't exist
/// (Node throws 'Ingestion source not found' there).
pub fn group_source_ids(conn: &Connection, source_id: &str) -> Option<Vec<String>> {
    let (id, merged): (String, Option<String>) = conn
        .query_row(
            "SELECT id, merged_into_id FROM ingestion_sources WHERE id = ?",
            [source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    let root_id = merged.unwrap_or(id);
    let mut ids: Vec<String> = vec![root_id.clone()];
    let mut stmt = conn
        .prepare("SELECT id FROM ingestion_sources WHERE merged_into_id = ?")
        .unwrap();
    let children = stmt
        .query_map([&root_id], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok);
    ids.extend(children);
    Some(ids)
}

/// Port of buildFilterSql — parameterized SQL over archived_emails (alias ae).
pub fn build_filter_sql(conn: &Connection, q: &dyn Fn(&str) -> Option<String>) -> FilterSql {
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();

    if let Some(source_id) = q("ingestionSourceId") {
        if let Some(ids) = group_source_ids(conn, &source_id) {
            let placeholders = vec!["?"; ids.len()].join(", ");
            parts.push(format!("ae.ingestion_source_id IN ({placeholders})"));
            params.extend(ids.into_iter().map(SqlValue::from));
        }
    }
    for (key, column) in [
        ("userEmail", "ae.user_email"),
        ("from", "ae.sender_email"),
        ("sourcePath", "ae.source_path"),
    ] {
        if let Some(value) = q(key) {
            if !value.is_empty() {
                parts.push(format!("{column} = ?"));
                params.push(SqlValue::from(value));
            }
        }
    }
    for key in ["to", "cc", "bcc"] {
        if let Some(value) = q(key) {
            if !value.is_empty() {
                parts.push(
                    "EXISTS (SELECT 1 FROM json_each(ae.recipients, '$.' || ?) r WHERE json_extract(r.value, '$.address') = ?)"
                        .to_string(),
                );
                params.push(SqlValue::from(key.to_string()));
                params.push(SqlValue::from(value));
            }
        }
    }
    if let Some(value) = q("hasAttachments") {
        if value == "true" || value == "false" {
            parts.push("ae.has_attachments = ?".to_string());
            params.push(SqlValue::from(if value == "true" { 1i64 } else { 0 }));
        }
    }
    for (key, column) in [("sourceLabels", "ae.source_labels"), ("tags", "ae.tags")] {
        if let Some(value) = q(key) {
            if !value.is_empty() {
                parts.push(format!(
                    "EXISTS (SELECT 1 FROM json_each(COALESCE({column}, '[]')) j WHERE j.value = ?)"
                ));
                params.push(SqlValue::from(value));
            }
        }
    }
    for (key, clause) in [
        ("sentAfter", "ae.sent_at >= ?"),
        ("sentBefore", "ae.sent_at <= ?"),
        ("archivedAfter", "ae.archived_at >= ?"),
        ("archivedBefore", "ae.archived_at <= ?"),
    ] {
        if let Some(ts) = to_timestamp(q(key).as_deref()) {
            parts.push(clause.to_string());
            params.push(SqlValue::from(ts));
        }
    }

    FilterSql {
        clause: if parts.is_empty() {
            String::new()
        } else {
            format!(" AND {}", parts.join(" AND "))
        },
        params,
    }
}

fn addresses(recipients: &Value, key: &str) -> Value {
    let list = recipients
        .get(key)
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("address").and_then(|a| a.as_str()))
                .filter(|a| !a.is_empty())
                .map(|a| Value::String(a.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(list)
}

fn parse_json_or(row_value: Option<String>, default: Value) -> Value {
    row_value
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(default)
}

/// Port of rowToDocument — the EmailDocument JSON shape, key order preserved.
pub fn row_to_document(row: &rusqlite::Row<'_>, snippet: Option<String>) -> Value {
    let recipients: Value =
        parse_json_or(row.get::<_, Option<String>>("recipients").unwrap_or(None), json!({}));
    let mut doc = Map::new();
    doc.insert("id".into(), json!(row.get::<_, String>("id").unwrap_or_default()));
    doc.insert(
        "userEmail".into(),
        json!(row.get::<_, String>("user_email").unwrap_or_default()),
    );
    doc.insert(
        "from".into(),
        json!(row.get::<_, String>("sender_email").unwrap_or_default()),
    );
    doc.insert(
        "senderName".into(),
        json!(row
            .get::<_, Option<String>>("sender_name")
            .unwrap_or(None)
            .unwrap_or_default()),
    );
    doc.insert("to".into(), addresses(&recipients, "to"));
    doc.insert("cc".into(), addresses(&recipients, "cc"));
    doc.insert("bcc".into(), addresses(&recipients, "bcc"));
    doc.insert(
        "subject".into(),
        json!(row
            .get::<_, Option<String>>("subject")
            .unwrap_or(None)
            .unwrap_or_default()),
    );
    doc.insert("body".into(), json!(snippet.unwrap_or_default()));
    doc.insert("attachments".into(), json!([]));
    doc.insert(
        "timestamp".into(),
        json!(row.get::<_, i64>("sent_at").unwrap_or_default()),
    );
    doc.insert(
        "archivedAt".into(),
        json!(row.get::<_, i64>("archived_at").unwrap_or_default()),
    );
    doc.insert(
        "ingestionSourceId".into(),
        json!(row.get::<_, String>("ingestion_source_id").unwrap_or_default()),
    );
    doc.insert(
        "threadId".into(),
        json!(row.get::<_, Option<String>>("thread_id").unwrap_or(None)),
    );
    doc.insert(
        "messageIdHeader".into(),
        json!(row.get::<_, Option<String>>("message_id_header").unwrap_or(None)),
    );
    doc.insert(
        "hasAttachments".into(),
        json!(row.get::<_, i64>("has_attachments").unwrap_or(0) != 0),
    );
    doc.insert(
        "sourcePath".into(),
        json!(row.get::<_, Option<String>>("source_path").unwrap_or(None)),
    );
    doc.insert(
        "sourceLabels".into(),
        parse_json_or(row.get::<_, Option<String>>("source_labels").unwrap_or(None), json!([])),
    );
    doc.insert(
        "tags".into(),
        parse_json_or(row.get::<_, Option<String>>("tags").unwrap_or(None), json!([])),
    );
    doc.insert(
        "sizeBytes".into(),
        json!(row.get::<_, i64>("size_bytes").unwrap_or_default()),
    );
    Value::Object(doc)
}

/// Port of queryArchivedEmails. `q` looks up a query parameter by name.
pub fn query_archived_emails(
    conn: &Connection,
    q: &dyn Fn(&str) -> Option<String>,
    started_ms: i64,
) -> Value {
    let query = q("q").or_else(|| q("query")).unwrap_or_default().trim().to_string();
    let page = clamp_positive(q("page").as_deref(), 1, i64::MAX);
    let limit = clamp_positive(q("limit").as_deref(), 10, 100);
    let fields = normalize_fields(q("fields").as_deref());
    let sort_col = sort_column(q("sort").as_deref());
    let direction = if q("direction").as_deref() == Some("asc") {
        "ASC"
    } else {
        "DESC"
    };
    let strict = q("matchingStrategy").as_deref() == Some("all");
    let filter = build_filter_sql(conn, q);

    let run = |match_expr: Option<&str>| -> (Vec<Value>, i64) {
        if let Some(expr) = match_expr {
            let base = format!(
                "FROM email_fts f JOIN archived_emails ae ON ae.rowid = f.rowid WHERE email_fts MATCH ?{}",
                filter.clause
            );
            let sql = format!(
                "SELECT ae.*, snippet(email_fts, 2, '', '', '…', 24) AS snippet {base} \
                 ORDER BY {sort_col} {direction}, bm25(email_fts, {BM25_WEIGHTS}) ASC LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let mut params: Vec<SqlValue> = vec![SqlValue::from(expr.to_string())];
            params.extend(filter.params.iter().cloned());
            params.push(SqlValue::from(limit));
            params.push(SqlValue::from((page - 1) * limit));
            let hits: Vec<Value> = stmt
                .query_map(rusqlite::params_from_iter(params), |row| {
                    let snippet: Option<String> = row.get("snippet").ok();
                    Ok(row_to_document(row, snippet))
                })
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            let count_sql = format!("SELECT count(*) {base}");
            let mut count_params: Vec<SqlValue> = vec![SqlValue::from(expr.to_string())];
            count_params.extend(filter.params.iter().cloned());
            let total: i64 = conn
                .query_row(&count_sql, rusqlite::params_from_iter(count_params), |r| r.get(0))
                .unwrap_or(0);
            (hits, total)
        } else {
            let base = format!("FROM archived_emails ae WHERE 1=1{}", filter.clause);
            let sql = format!(
                "SELECT ae.* {base} ORDER BY {sort_col} {direction} LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let mut params: Vec<SqlValue> = filter.params.clone();
            params.push(SqlValue::from(limit));
            params.push(SqlValue::from((page - 1) * limit));
            let hits: Vec<Value> = stmt
                .query_map(rusqlite::params_from_iter(params), |row| {
                    Ok(row_to_document(row, None))
                })
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            let count_sql = format!("SELECT count(*) {base}");
            let total: i64 = conn
                .query_row(
                    &count_sql,
                    rusqlite::params_from_iter(filter.params.iter().cloned()),
                    |r| r.get(0),
                )
                .unwrap_or(0);
            (hits, total)
        }
    };

    let mut result = run(
        (!query.is_empty())
            .then(|| build_match(&query, &fields, false))
            .flatten()
            .as_deref(),
    );
    if !query.is_empty() && result.1 == 0 && !strict {
        if let Some(or_match) = build_match(&query, &fields, true) {
            if or_match.contains(" OR ") {
                result = run(Some(&or_match));
            }
        }
    }

    let (hits, total) = result;
    json!({
        "hits": hits,
        "total": total,
        "page": page,
        "limit": limit,
        "totalPages": (total as f64 / limit as f64).ceil() as i64,
        "processingTimeMs": now_ms() - started_ms,
    })
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// getTopSenders — dashboard facet.
pub fn top_senders(conn: &Connection, limit: i64) -> Value {
    let mut stmt = conn
        .prepare(
            "SELECT sender_email AS sender, count(*) AS count FROM archived_emails \
             GROUP BY sender_email ORDER BY count DESC, sender ASC LIMIT ?",
        )
        .unwrap();
    let rows: Vec<Value> = stmt
        .query_map([limit], |row| {
            Ok(json!({
                "sender": row.get::<_, String>(0)?,
                "count": row.get::<_, i64>(1)?,
            }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Value::Array(rows)
}

/// getFilterFacets — distinct tags, uncapped.
pub fn filter_facets(conn: &Connection) -> Value {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT j.value AS tag FROM archived_emails ae, \
             json_each(COALESCE(ae.tags, '[]')) j WHERE trim(j.value) <> '' \
             ORDER BY tag COLLATE NOCASE ASC",
        )
        .unwrap();
    let tags: Vec<Value> = stmt
        .query_map([], |row| Ok(Value::String(row.get::<_, String>(0)?)))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    json!({ "tags": tags })
}
