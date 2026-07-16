//! Duplicate review, read side.
//!
//! The user-facing model deliberately has only two classifications:
//! - exact: safe to bulk-delete because identity, content, recipients and
//!   attachment metadata all agree (or the stored EML bytes are identical);
//! - likely: the semantic message is identical, but the provider Message-ID or
//!   attachment metadata differs, so the group requires individual review.
//!
//! Likely matching is intentionally conservative. Sender, recipients, sent
//! time, normalized subject/body and attachment bytes must all be equal. That
//! excludes reply chains, rapid back-and-forth messages, and forwards to other
//! recipients instead of trying to score their textual similarity.

use crate::iso;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

const DEFAULT_LIMIT: i64 = 25;
const MAX_LIMIT: i64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DuplicateClassification {
    Exact,
    Likely,
}

impl DuplicateClassification {
    fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Likely => "likely",
        }
    }
}

#[derive(Clone)]
pub(crate) struct DuplicateCluster {
    pub ids: Vec<String>,
    pub min_id: String,
    pub classification: DuplicateClassification,
    pub default_keeper_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheRevision {
    total_emails: i64,
    active_emails: i64,
    latest_archived_at: i64,
    latest_deleted_at: i64,
    ignored_groups: i64,
    latest_ignore_at: i64,
}

#[derive(Default)]
pub struct DuplicateCache {
    revision: Option<CacheRevision>,
    clusters: Vec<DuplicateCluster>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SemanticKey {
    subject_hash: Option<String>,
    sender_email: String,
    recipient_fingerprint: Option<String>,
    sent_at: i64,
    body_hash: Option<String>,
    attachment_content_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ExactKey {
    semantic: SemanticKey,
    message_id: String,
    attachment_metadata_fingerprint: String,
}

struct SignalRow {
    id: String,
    sent_at: i64,
    archived_at: i64,
    storage_hash: String,
    message_id: Option<String>,
    semantic: SemanticKey,
    attachment_metadata_fingerprint: String,
}

fn clamp_positive(value: Option<&str>, fallback: i64, max: i64) -> i64 {
    let parsed = value.and_then(|v| v.parse::<f64>().ok());
    match parsed {
        Some(n) if n.is_finite() && n >= 1.0 => (n.floor() as i64).min(max),
        _ => fallback,
    }
}

fn map_email(row: &rusqlite::Row) -> rusqlite::Result<Map<String, Value>> {
    let mut doc = Map::new();
    doc.insert("id".into(), json!(row.get::<_, String>("id")?));
    doc.insert(
        "subject".into(),
        json!(row.get::<_, Option<String>>("subject")?),
    );
    doc.insert(
        "senderName".into(),
        json!(row.get::<_, Option<String>>("sender_name")?),
    );
    doc.insert(
        "senderEmail".into(),
        json!(row.get::<_, String>("sender_email")?),
    );
    doc.insert(
        "importSource".into(),
        json!(row.get::<_, String>("import_source")?),
    );
    doc.insert("sentAt".into(), json!(iso(row.get::<_, i64>("sent_at")?)));
    doc.insert(
        "archivedAt".into(),
        json!(iso(row.get::<_, i64>("archived_at")?)),
    );
    doc.insert(
        "hasAttachments".into(),
        json!(row.get::<_, i64>("has_attachments")? != 0),
    );
    doc.insert(
        "sourcePath".into(),
        json!(row.get::<_, Option<String>>("source_path")?),
    );
    doc.insert(
        "messageIdHeader".into(),
        json!(row.get::<_, Option<String>>("message_id_header")?),
    );
    doc.insert(
        "storageHashSha256".into(),
        json!(row.get::<_, String>("storage_hash_sha256")?),
    );
    Ok(doc)
}

fn find_emails_by_ids(conn: &Connection, ids: &[String]) -> Vec<Value> {
    if ids.is_empty() {
        return Vec::new();
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "SELECT ae.id, ae.subject, ae.sender_name, ae.sender_email, \
         COALESCE(src.name, ae.import_source) AS import_source, ae.sent_at, ae.archived_at, \
         ae.has_attachments, ae.source_path, ae.message_id_header, ae.storage_hash_sha256 \
         FROM archived_emails ae \
         LEFT JOIN ingestion_sources src ON src.id = ae.ingestion_source_id \
         WHERE ae.id IN ({placeholders}) \
         ORDER BY ae.sent_at ASC, ae.archived_at ASC, ae.id ASC"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok(Value::Object(map_email(row)?))
    })
    .unwrap()
    .filter_map(Result::ok)
    .collect()
}

/// Length-prefix a field so filenames containing separators cannot make two
/// different attachment metadata sets compare equal.
fn append_fingerprint_field(out: &mut String, value: &str) {
    out.push_str(&value.len().to_string());
    out.push(':');
    out.push_str(value);
}

fn load_attachment_metadata_fingerprints(conn: &Connection) -> HashMap<String, String> {
    let mut by_email: HashMap<String, String> = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT ea.email_id, a.content_hash_sha256, a.filename, \
                    coalesce(a.mime_type, ''), a.size_bytes \
             FROM email_attachments ea \
             JOIN attachments a ON a.id = ea.attachment_id \
             JOIN archived_emails ae ON ae.id = ea.email_id \
             WHERE ae.deleted_at IS NULL \
             ORDER BY ea.email_id, a.content_hash_sha256, a.filename, \
                      coalesce(a.mime_type, ''), a.size_bytes",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .unwrap();
    for row in rows.flatten() {
        let entry = by_email.entry(row.0).or_default();
        append_fingerprint_field(entry, &row.1);
        append_fingerprint_field(entry, &row.2);
        append_fingerprint_field(entry, &row.3);
        append_fingerprint_field(entry, &row.4.to_string());
    }
    by_email
}

fn load_signal_rows(conn: &Connection) -> Vec<SignalRow> {
    let mut attachment_metadata = load_attachment_metadata_fingerprints(conn);
    let mut stmt = conn
        .prepare(
            "SELECT id, sent_at, archived_at, storage_hash_sha256, \
                    nullif(trim(message_id_header), ''), duplicate_subject_hash, \
                    lower(coalesce(sender_email, '')), duplicate_recipient_fingerprint, \
                    duplicate_body_hash, duplicate_attachment_fingerprint \
             FROM archived_emails \
             WHERE deleted_at IS NULL",
        )
        .unwrap();
    stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let sent_at = row.get(1)?;
        Ok(SignalRow {
            attachment_metadata_fingerprint: attachment_metadata.remove(&id).unwrap_or_default(),
            semantic: SemanticKey {
                subject_hash: row.get(5)?,
                sender_email: row.get(6)?,
                recipient_fingerprint: row.get(7)?,
                sent_at,
                body_hash: row.get(8)?,
                attachment_content_fingerprint: row.get(9)?,
            },
            id,
            sent_at,
            archived_at: row.get(2)?,
            storage_hash: row.get(3)?,
            message_id: row.get(4)?,
        })
    })
    .unwrap()
    .filter_map(Result::ok)
    .collect()
}

fn find(parent: &mut [usize], x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    let mut current = x;
    while parent[current] != root {
        let next = parent[current];
        parent[current] = root;
        current = next;
    }
    root
}

fn union_members(parent: &mut [usize], members: &[usize]) {
    for member in members.iter().skip(1) {
        let left = find(parent, members[0]);
        let right = find(parent, *member);
        if left != right {
            parent[left] = right;
        }
    }
}

/// Computes all groups in approximately O(emails × indexed signals). Exact
/// components are formed first; a semantic bucket containing more than one
/// exact component becomes one conservative Likely group.
pub(crate) fn collect_duplicate_clusters(conn: &Connection) -> Vec<DuplicateCluster> {
    let rows = load_signal_rows(conn);
    let mut parent: Vec<usize> = (0..rows.len()).collect();

    let mut by_storage_hash: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut by_exact_key: HashMap<ExactKey, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if !row.storage_hash.is_empty() {
            by_storage_hash
                .entry(&row.storage_hash)
                .or_default()
                .push(index);
        }
        if let Some(message_id) = &row.message_id {
            by_exact_key
                .entry(ExactKey {
                    semantic: row.semantic.clone(),
                    message_id: message_id.to_lowercase(),
                    attachment_metadata_fingerprint: row.attachment_metadata_fingerprint.clone(),
                })
                .or_default()
                .push(index);
        }
    }
    for members in by_storage_hash.values().chain(by_exact_key.values()) {
        union_members(&mut parent, members);
    }

    let mut exact_components: HashMap<usize, Vec<usize>> = HashMap::new();
    for index in 0..rows.len() {
        let root = find(&mut parent, index);
        exact_components.entry(root).or_default().push(index);
    }

    let mut by_semantic_key: HashMap<&SemanticKey, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        by_semantic_key
            .entry(&row.semantic)
            .or_default()
            .push(index);
    }

    let ignored = load_ignored_fingerprints(conn);
    let mut exact_roots_in_likely_groups: HashSet<usize> = HashSet::new();
    let mut clusters: Vec<DuplicateCluster> = Vec::new();

    for members in by_semantic_key.values() {
        if members.len() < 2 {
            continue;
        }
        let roots: HashSet<usize> = members.iter().map(|i| find(&mut parent, *i)).collect();
        if roots.len() < 2 {
            continue;
        }
        exact_roots_in_likely_groups.extend(roots);
        if let Some(cluster) =
            build_cluster(&rows, members, DuplicateClassification::Likely, &ignored)
        {
            clusters.push(cluster);
        }
    }

    for (root, members) in exact_components {
        if exact_roots_in_likely_groups.contains(&root) || members.len() < 2 {
            continue;
        }
        if let Some(cluster) =
            build_cluster(&rows, &members, DuplicateClassification::Exact, &ignored)
        {
            clusters.push(cluster);
        }
    }

    clusters
}

fn build_cluster(
    rows: &[SignalRow],
    members: &[usize],
    classification: DuplicateClassification,
    ignored: &HashSet<String>,
) -> Option<DuplicateCluster> {
    let mut member_rows: Vec<&SignalRow> = members.iter().map(|i| &rows[*i]).collect();
    member_rows.sort_by(|a, b| {
        a.sent_at
            .cmp(&b.sent_at)
            .then_with(|| a.archived_at.cmp(&b.archived_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let ids: Vec<String> = member_rows.iter().map(|row| row.id.clone()).collect();
    let min_id = ids.iter().min()?.clone();
    if ignored.contains(&min_id) {
        return None;
    }
    Some(DuplicateCluster {
        default_keeper_id: member_rows.first()?.id.clone(),
        ids,
        min_id,
        classification,
    })
}

pub(crate) fn collect_exact_clusters(conn: &Connection) -> Vec<DuplicateCluster> {
    collect_duplicate_clusters(conn)
        .into_iter()
        .filter(|cluster| cluster.classification == DuplicateClassification::Exact)
        .collect()
}

fn cache_revision(conn: &Connection) -> CacheRevision {
    let email_revision = conn
        .query_row(
            "SELECT count(*), \
                    sum(CASE WHEN deleted_at IS NULL THEN 1 ELSE 0 END), \
                    coalesce(max(archived_at), 0), coalesce(max(deleted_at), 0) \
             FROM archived_emails",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    let ignore_revision = conn
        .query_row(
            "SELECT count(*), coalesce(max(created_at), 0) FROM exact_duplicate_ignores",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0, 0));
    CacheRevision {
        total_emails: email_revision.0,
        active_emails: email_revision.1,
        latest_archived_at: email_revision.2,
        latest_deleted_at: email_revision.3,
        ignored_groups: ignore_revision.0,
        latest_ignore_at: ignore_revision.1,
    }
}

fn cached_clusters(
    conn: &Connection,
    cache: &std::sync::Mutex<DuplicateCache>,
) -> Vec<DuplicateCluster> {
    let revision = cache_revision(conn);
    if let Ok(guard) = cache.lock() {
        if guard.revision.as_ref() == Some(&revision) {
            return guard.clusters.clone();
        }
    }
    let clusters = collect_duplicate_clusters(conn);
    if let Ok(mut guard) = cache.lock() {
        guard.revision = Some(revision);
        guard.clusters = clusters.clone();
    }
    clusters
}

pub fn list_duplicate_groups(
    conn: &Connection,
    cache: &std::sync::Mutex<DuplicateCache>,
    page: Option<&str>,
    limit: Option<&str>,
    classification: Option<&str>,
) -> Value {
    let normalized_page = clamp_positive(page, 1, i64::MAX);
    let normalized_limit = clamp_positive(limit, DEFAULT_LIMIT, MAX_LIMIT);
    let offset = (normalized_page - 1).saturating_mul(normalized_limit) as usize;
    let classification = match classification {
        Some("exact") => Some(DuplicateClassification::Exact),
        Some("likely") => Some(DuplicateClassification::Likely),
        _ => None,
    };

    let clusters = cached_clusters(conn, cache);
    let exact_count = clusters
        .iter()
        .filter(|cluster| cluster.classification == DuplicateClassification::Exact)
        .count();
    let likely_count = clusters.len() - exact_count;
    let mut filtered: Vec<&DuplicateCluster> = clusters
        .iter()
        .filter(|cluster| classification.map_or(true, |value| cluster.classification == value))
        .collect();
    filtered.sort_by(|a, b| {
        b.ids
            .len()
            .cmp(&a.ids.len())
            .then_with(|| a.min_id.cmp(&b.min_id))
    });

    let total_groups = filtered.len();
    let mut groups: Vec<Value> = Vec::new();
    for cluster in filtered
        .into_iter()
        .skip(offset)
        .take(normalized_limit as usize)
    {
        let emails = find_emails_by_ids(conn, &cluster.ids);
        if emails.len() <= 1 || cluster.default_keeper_id.is_empty() {
            continue;
        }
        groups.push(json!({
            "groupKey": format!("cluster:{}", cluster.min_id),
            "classification": cluster.classification.as_str(),
            "fingerprint": cluster.min_id,
            "count": emails.len(),
            "keeperEmailId": cluster.default_keeper_id,
            "emails": emails,
        }));
    }

    json!({
        "groups": groups,
        "totalGroups": total_groups,
        "classificationCounts": {
            "all": clusters.len(),
            "exact": exact_count,
            "likely": likely_count,
        },
        "page": normalized_page,
        "limit": normalized_limit,
    })
}

fn load_ignored_fingerprints(conn: &Connection) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT fingerprint FROM exact_duplicate_ignores") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for fingerprint in rows.flatten() {
                set.insert(fingerprint);
            }
        }
    }
    set
}
