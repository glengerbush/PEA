//! Full-text search over archived emails (SQLite FTS5).

use rusqlite::{types::Value as SqlValue, Connection};
use serde_json::{json, Map, Value};

const ALL_COLUMNS: [&str; 6] = [
    "subject",
    "body",
    "sender",
    "recipients",
    "attachments",
    "meta",
];
// bm25 weights per FTS column order: (email_id, subject, body, sender, recipients, attachments, meta)
const BM25_WEIGHTS: &str = "0.0, 10.0, 2.0, 6.0, 3.0, 1.0, 2.0";

fn field_to_column(field: &str) -> Option<&'static str> {
    match field {
        "subject" => Some("subject"),
        "body" => Some("body"),
        "from" | "senderName" => Some("sender"),
        "to" | "cc" | "bcc" => Some("recipients"),
        "attachments.filename" | "attachments.content" => Some("attachments"),
        // importSource searches the FTS `meta` column, which is indexed at
        // ingest time from the filename-derived archived_emails.import_source —
        // NOT the live ingestion-source name shown in listings (reindexing on
        // every source rename isn't worth it).
        "importSource" | "sourcePath" | "tags" => Some("meta"),
        _ => None,
    }
}

const DEFAULT_FIELDS: [&str; 12] = [
    "subject",
    "body",
    "from",
    "senderName",
    "to",
    "cc",
    "bcc",
    "attachments.filename",
    "attachments.content",
    "importSource",
    "sourcePath",
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

    let mut columns: Vec<&str> = fields.iter().filter_map(|f| field_to_column(f)).collect();
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
    let value = value?.trim();
    if let Ok(n) = value.parse::<i64>() {
        return Some(n);
    }
    parse_iso8601_ms(value)
}

/// Parse an API timestamp input (epoch-ms integer string or ISO-8601 date) to
/// epoch-ms. Public wrapper over the filter parser.
pub fn parse_timestamp(value: &str) -> Option<i64> {
    to_timestamp(Some(value))
}

/// Minimal ISO-8601 → epoch-milliseconds (UTC). Accepts "YYYY-MM-DD" and
/// "YYYY-MM-DD[T ]HH:MM[:SS]" with an optional trailing 'Z' / `+HH:MM` offset
/// (offsets and fractional seconds are treated as UTC — enough for the date
/// range filters). Returns None when the date is not a valid calendar date.
fn parse_iso8601_ms(s: &str) -> Option<i64> {
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut dp = date.split('-');
    let year: i64 = dp.next()?.parse().ok()?;
    let month: i64 = dp.next()?.parse().ok()?;
    let day: i64 = dp.next()?.parse().ok()?;
    // Bound the year to a sane range so the epoch arithmetic below can't overflow
    // on adversarial input (e.g. "300000000-01-01").
    if dp.next().is_some() || !(1..=12).contains(&month) || !(1..=9999).contains(&year) {
        return None;
    }
    let (mut hh, mut mm, mut ss) = (0i64, 0i64, 0i64);
    if let Some(t) = time {
        // drop trailing 'Z', a +/- timezone offset, and fractional seconds
        // (the HH:MM[:SS] portion has no '-' except in an offset).
        let t = t.trim_end_matches('Z');
        let t = &t[..t.find(['+', '-']).unwrap_or(t.len())];
        let t = t.split('.').next().unwrap_or(t);
        let mut tp = t.split(':');
        hh = tp.next().unwrap_or("0").parse().ok()?;
        mm = tp.next().unwrap_or("0").parse().ok()?;
        ss = tp.next().unwrap_or("0").parse().ok()?;
    }
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    Some((days * 86_400 + hh * 3_600 + mm * 60 + ss) * 1_000)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's algorithm),
/// validating the day-of-month. None for an impossible date (e.g. Feb 30).
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if d < 1 || d > mdays[(m - 1) as usize] {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Source ids in a merge group — root = merged_into_id ?? id, plus all
/// children of the root. `None` when the source id doesn't exist (the caller
/// maps that to a 500).
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

/// Builds parameterized SQL over archived_emails (alias ae).
pub fn build_filter_sql(conn: &Connection, q: &dyn Fn(&str) -> Option<String>) -> FilterSql {
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();

    // Trash: the default listing hides trashed emails; `?trashed=true` shows only
    // them (the Trash view). This clause is always present, so every count/list
    // path that runs through build_filter_sql excludes trashed rows by default.
    if q("trashed").as_deref() == Some("true") {
        parts.push("ae.deleted_at IS NOT NULL".to_string());
    } else {
        parts.push("ae.deleted_at IS NULL".to_string());
    }

    if let Some(source_id) = q("ingestionSourceId") {
        if !source_id.is_empty() {
            match group_source_ids(conn, &source_id) {
                Some(ids) => {
                    let placeholders = vec!["?"; ids.len()].join(", ");
                    parts.push(format!("ae.ingestion_source_id IN ({placeholders})"));
                    params.extend(ids.into_iter().map(SqlValue::from));
                }
                // A provided-but-unknown source scopes to nothing — it must not
                // fall through to "no filter" and return every email.
                None => parts.push("1 = 0".to_string()),
            }
        }
    }
    for (key, column) in [
        // Filters on the raw ingest-time value, not the live source name
        // (listings display the latter; use ingestionSourceId to filter by source).
        ("importSource", "ae.import_source"),
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
    if let Some(value) = q("tags") {
        if !value.is_empty() {
            parts.push(
                "EXISTS (SELECT 1 FROM json_each(COALESCE(ae.tags, '[]')) j WHERE j.value = ?)"
                    .to_string(),
            );
            params.push(SqlValue::from(value));
        }
    }
    // Comma-separated attachment filename extensions (any-of).
    if let Some(value) = q("attachmentExt") {
        let exts: Vec<String> = value
            .split(',')
            .map(|e| e.trim().trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .collect();
        if !exts.is_empty() {
            let likes = vec!["a2.filename LIKE ?"; exts.len()].join(" OR ");
            parts.push(format!(
                "EXISTS (SELECT 1 FROM email_attachments ea2 \
                 INNER JOIN attachments a2 ON ea2.attachment_id = a2.id \
                 WHERE ea2.email_id = ae.id AND ({likes}))"
            ));
            params.extend(exts.into_iter().map(|e| SqlValue::from(format!("%.{e}"))));
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

/// Builds the EmailDocument JSON shape, key order preserved.
pub fn row_to_document(row: &rusqlite::Row<'_>, snippet: Option<String>) -> Value {
    let recipients: Value = parse_json_or(
        row.get::<_, Option<String>>("recipients").unwrap_or(None),
        json!({}),
    );
    let mut doc = Map::new();
    doc.insert(
        "id".into(),
        json!(row.get::<_, String>("id").unwrap_or_default()),
    );
    // Prefer the live ingestion-source name (joined as import_source_name) so
    // renames in the Import tab show immediately; fall back to the
    // filename-derived string frozen at ingest.
    let import_source = row
        .get::<_, Option<String>>("import_source_name")
        .ok()
        .flatten()
        .or_else(|| row.get::<_, String>("import_source").ok())
        .unwrap_or_default();
    doc.insert("importSource".into(), json!(import_source));
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
    // What `timestamp` actually is, so the list can label it honestly. NULL
    // (pre-backfill rows) reads as "sent" — the historical default.
    doc.insert(
        "timestampKind".into(),
        json!(row
            .get::<_, Option<String>>("sent_at_kind")
            .ok()
            .flatten()
            .unwrap_or_else(|| "sent".into())),
    );
    doc.insert(
        "archivedAt".into(),
        json!(row.get::<_, i64>("archived_at").unwrap_or_default()),
    );
    doc.insert(
        "ingestionSourceId".into(),
        json!(row
            .get::<_, String>("ingestion_source_id")
            .unwrap_or_default()),
    );
    doc.insert(
        "threadId".into(),
        json!(row.get::<_, Option<String>>("thread_id").unwrap_or(None)),
    );
    doc.insert(
        "messageIdHeader".into(),
        json!(row
            .get::<_, Option<String>>("message_id_header")
            .unwrap_or(None)),
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
        "tags".into(),
        parse_json_or(
            row.get::<_, Option<String>>("tags").unwrap_or(None),
            json!([]),
        ),
    );
    doc.insert(
        "sizeBytes".into(),
        json!(row.get::<_, i64>("size_bytes").unwrap_or_default()),
    );
    Value::Object(doc)
}

/// Runs the archived-email search/listing query. `q` looks up a query parameter by name.
pub fn query_archived_emails(
    conn: &Connection,
    q: &dyn Fn(&str) -> Option<String>,
    started_ms: i64,
) -> Value {
    let query = q("q")
        .or_else(|| q("query"))
        .unwrap_or_default()
        .trim()
        .to_string();
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
    let field_match_any = q("fieldMatch").as_deref() == Some("any");
    let filter = build_filter_sql(conn, q);

    // The archive UI exposes a compose-like advanced search. Each populated
    // input becomes its own field-scoped FTS clause; fieldMatch controls how
    // those clauses (and the all-fields query) relate to one another. Keeping
    // this in MATCH rather than SQL filters preserves prefix search and names.
    let mut text_clauses: Vec<String> = Vec::new();
    if let Some(expr) = build_match(&query, &fields, false) {
        text_clauses.push(expr);
    }
    for (key, scoped_fields) in [
        ("senderQuery", &["from", "senderName"][..]),
        ("recipientsQuery", &["to", "cc", "bcc"][..]),
        ("subjectQuery", &["subject"][..]),
        ("bodyQuery", &["body"][..]),
    ] {
        let value = q(key).unwrap_or_default();
        let value = value.trim();
        let scoped_fields: Vec<String> =
            scoped_fields.iter().map(|field| (*field).into()).collect();
        if let Some(expr) = build_match(value, &scoped_fields, false) {
            text_clauses.push(expr);
        }
    }
    let match_expr = (!text_clauses.is_empty()).then(|| {
        let joiner = if field_match_any { " OR " } else { " AND " };
        text_clauses
            .into_iter()
            .map(|clause| format!("({clause})"))
            .collect::<Vec<_>>()
            .join(joiner)
    });

    let run = |match_expr: Option<&str>| -> (Vec<Value>, i64) {
        if let Some(expr) = match_expr {
            let base = format!(
                "FROM email_fts f JOIN archived_emails ae ON ae.rowid = f.rowid \
                 LEFT JOIN ingestion_sources src ON src.id = ae.ingestion_source_id \
                 WHERE email_fts MATCH ?{}",
                filter.clause
            );
            // The total is a SEPARATE count query. A window function
            // (COUNT(*) OVER ()) cannot share a SELECT with the FTS5 auxiliary
            // functions bm25()/snippet() — SQLite raises "unable to use function
            // bm25 in the requested context", which (swallowed by the row-level
            // filter_map below) silently zeroed every page-1 search. The count
            // is JOIN-free when unfiltered, so it only walks the FTS doclist.
            let sql = format!(
                "SELECT ae.*, COALESCE(src.name, ae.import_source) AS import_source_name, \
                 snippet(email_fts, 2, '', '', '…', 24) AS snippet {base} \
                 ORDER BY {sort_col} {direction}, bm25(email_fts, {BM25_WEIGHTS}) ASC LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let mut params: Vec<SqlValue> = vec![SqlValue::from(expr.to_string())];
            params.extend(filter.params.iter().cloned());
            params.push(SqlValue::from(limit));
            params.push(SqlValue::from((page - 1).saturating_mul(limit)));
            let hits: Vec<Value> = stmt
                .query_map(rusqlite::params_from_iter(params), |row| {
                    let snippet: Option<String> = row.get("snippet").ok();
                    Ok(row_to_document(row, snippet))
                })
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            let count_base = if filter.clause.is_empty() {
                "FROM email_fts WHERE email_fts MATCH ?".to_string()
            } else {
                base.clone()
            };
            let count_sql = format!("SELECT count(*) {count_base}");
            let mut count_params: Vec<SqlValue> = vec![SqlValue::from(expr.to_string())];
            count_params.extend(filter.params.iter().cloned());
            let total: i64 = conn
                .query_row(&count_sql, rusqlite::params_from_iter(count_params), |r| {
                    r.get(0)
                })
                .unwrap_or(0);
            (hits, total)
        } else {
            let base = format!(
                "FROM archived_emails ae \
                 LEFT JOIN ingestion_sources src ON src.id = ae.ingestion_source_id \
                 WHERE 1=1{}",
                filter.clause
            );
            let sql = format!(
                "SELECT ae.*, COALESCE(src.name, ae.import_source) AS import_source_name \
                 {base} ORDER BY {sort_col} {direction} LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let mut params: Vec<SqlValue> = filter.params.clone();
            params.push(SqlValue::from(limit));
            params.push(SqlValue::from((page - 1).saturating_mul(limit)));
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

    let has_scoped_queries = [
        "senderQuery",
        "recipientsQuery",
        "subjectQuery",
        "bodyQuery",
    ]
    .iter()
    .any(|key| q(key).is_some_and(|value| !value.trim().is_empty()));
    let mut result = run(match_expr.as_deref());
    // Preserve the original forgiving OR fallback for the simple, all-fields
    // search. Advanced field clauses are explicit and should never silently
    // broaden when their intersection is empty.
    if !query.is_empty() && !has_scoped_queries && result.1 == 0 && !strict {
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
             WHERE deleted_at IS NULL \
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
             json_each(COALESCE(ae.tags, '[]')) j \
             WHERE trim(j.value) <> '' AND ae.deleted_at IS NULL \
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_parse_to_epoch_ms() {
        assert_eq!(parse_iso8601_ms("2024-01-01"), Some(1_704_067_200_000));
        assert_eq!(
            parse_iso8601_ms("2024-01-01T00:00:00Z"),
            Some(1_704_067_200_000)
        );
        assert_eq!(parse_iso8601_ms("1970-01-01"), Some(0));
        assert_eq!(
            parse_iso8601_ms("2024-01-01T01:00:00"),
            Some(1_704_070_800_000)
        );
        assert_eq!(parse_iso8601_ms("2024-13-01"), None);
        assert_eq!(parse_iso8601_ms("2024-02-30"), None);
        assert_eq!(parse_iso8601_ms("notadate"), None);
    }

    #[test]
    fn to_timestamp_accepts_epoch_and_iso() {
        assert_eq!(to_timestamp(Some("1704067200000")), Some(1_704_067_200_000));
        assert_eq!(to_timestamp(Some("2024-01-01")), Some(1_704_067_200_000));
        assert_eq!(to_timestamp(Some("")), None);
        assert_eq!(to_timestamp(None), None);
    }

    #[test]
    fn days_from_civil_validity() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(1970, 1, 2), Some(1));
        assert!(days_from_civil(2000, 2, 29).is_some());
        assert_eq!(days_from_civil(2001, 2, 29), None);
        assert_eq!(days_from_civil(2024, 4, 31), None);
    }

    #[test]
    fn build_match_prefixes_and_scopes() {
        let all = normalize_fields(None);
        assert_eq!(
            build_match("hello", &all, false).as_deref(),
            Some("\"hello\"*")
        );
        assert_eq!(
            build_match("foo bar", &all, false).as_deref(),
            Some("\"foo\" \"bar\"*")
        );
        assert_eq!(
            build_match("foo bar", &all, true).as_deref(),
            Some("\"foo\" OR \"bar\"*")
        );
        assert_eq!(
            build_match("hi", &vec!["subject".to_string()], false).as_deref(),
            Some("{subject} : (\"hi\"*)")
        );
        assert_eq!(build_match("   ", &all, false), None);
    }

    #[test]
    fn normalize_fields_defaults_and_filters() {
        assert_eq!(normalize_fields(None).len(), 12);
        assert_eq!(
            normalize_fields(Some("subject,body")),
            vec!["subject", "body"]
        );
        assert_eq!(normalize_fields(Some("bogus")).len(), 12);
    }

    #[test]
    fn clamp_positive_bounds() {
        assert_eq!(clamp_positive(Some("5"), 1, 100), 5);
        assert_eq!(clamp_positive(Some("0"), 1, 100), 1);
        assert_eq!(clamp_positive(Some("999"), 1, 100), 100);
        assert_eq!(clamp_positive(None, 7, 100), 7);
        assert_eq!(clamp_positive(Some("abc"), 3, 100), 3);
    }

    #[test]
    fn sort_and_field_column_mapping() {
        assert_eq!(sort_column(Some("subject")), "ae.subject");
        assert_eq!(sort_column(Some("bogus")), "ae.sent_at");
        assert_eq!(field_to_column("from"), Some("sender"));
        assert_eq!(field_to_column("tags"), Some("meta"));
        assert_eq!(field_to_column("bogus"), None);
    }
}
