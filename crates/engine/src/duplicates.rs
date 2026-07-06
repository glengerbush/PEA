//! Duplicate review, read side: exact-duplicate clustering via union-find over
//! several signals, computed on demand from archived_emails. Nothing about the
//! candidate set is materialized; only a user's *ignore* decisions persist
//! (exact_duplicate_ignores), keyed by the cluster fingerprint.

use crate::iso;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

const DEFAULT_LIMIT: i64 = 25;
const MAX_LIMIT: i64 = 100;

// (reason name, is_strong), strongest first — also the primary-reason order.
// STRONG signals form clusters (union-find). WEAK signals never link a cluster
// on their own; they only enrich the badges of a cluster that a strong signal
// already formed, so they can never be a group's sole reason. (Attachment sets
// are deduped in storage and message bodies can be identical across genuinely
// distinct newsletters, so neither is trustworthy as a lone match.)
const REASON_KEYS: [(&str, bool); 5] = [
    ("storage_hash", true),
    ("message_id", true),
    ("sender_recipients_sent", true),
    ("attachment_hash_set", false),
    ("message_body", false),
];

fn clamp_positive(value: Option<&str>, fallback: i64, max: i64) -> i64 {
    let parsed = value.and_then(|v| v.parse::<f64>().ok());
    match parsed {
        Some(n) if n.is_finite() && n >= 1.0 => (n.floor() as i64).min(max),
        _ => fallback,
    }
}

/// mapEmail — the shared per-email shape of both duplicate listings.
fn map_email(row: &rusqlite::Row) -> rusqlite::Result<Map<String, Value>> {
    let mut doc = Map::new();
    doc.insert("id".into(), json!(row.get::<_, String>("id")?));
    doc.insert("subject".into(), json!(row.get::<_, Option<String>>("subject")?));
    doc.insert("senderName".into(), json!(row.get::<_, Option<String>>("sender_name")?));
    doc.insert("senderEmail".into(), json!(row.get::<_, String>("sender_email")?));
    doc.insert("importSource".into(), json!(row.get::<_, String>("import_source")?));
    doc.insert("sentAt".into(), json!(iso(row.get::<_, i64>("sent_at")?)));
    doc.insert("archivedAt".into(), json!(iso(row.get::<_, i64>("archived_at")?)));
    doc.insert("hasAttachments".into(), json!(row.get::<_, i64>("has_attachments")? != 0));
    doc.insert("sourcePath".into(), json!(row.get::<_, Option<String>>("source_path")?));
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
        "SELECT id, subject, sender_name, sender_email, import_source, sent_at, archived_at, \
         has_attachments, source_path, message_id_header, storage_hash_sha256 \
         FROM archived_emails WHERE id IN ({placeholders}) \
         ORDER BY sent_at ASC, archived_at ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok(Value::Object(map_email(row)?))
    })
    .unwrap()
    .filter_map(Result::ok)
    .collect()
}

struct SignalRow {
    id: String,
    // per REASON_KEYS order: storage_hash, message_id, sender_recipients_sent,
    // attachment_hash_set, message_body
    values: [Option<String>; 5],
}

pub fn list_exact_groups(
    conn: &Connection,
    page: Option<&str>,
    limit: Option<&str>,
    reason: Option<&str>,
) -> Value {
    let normalized_page = clamp_positive(page, 1, i64::MAX);
    let normalized_limit = clamp_positive(limit, DEFAULT_LIMIT, MAX_LIMIT);
    // saturating_mul: a huge page number must yield an empty page, not overflow.
    let offset = (normalized_page - 1).saturating_mul(normalized_limit) as usize;

    let allowed: [&str; 5] = [
        "message_id",
        "storage_hash",
        "attachment_hash_set",
        "sender_recipients_sent",
        "message_body",
    ];
    let reason = reason.filter(|r| allowed.contains(r));

    // Pull every email's duplicate signals in one pass, then group by CONNECTED
    // COMPONENT (union-find), exactly like the Node implementation.
    let mut stmt = conn
        .prepare(
            "WITH attachment_sets AS ( \
                SELECT ae.id AS email_id, \
                    group_concat(a.content_hash_sha256, ',' ORDER BY a.content_hash_sha256) AS att_fp \
                FROM archived_emails ae \
                JOIN email_attachments ea ON ea.email_id = ae.id \
                JOIN attachments a ON a.id = ea.attachment_id \
                GROUP BY ae.id \
                HAVING count(a.id) > 0 \
            ) \
            SELECT ae.id AS id, \
                nullif(ae.message_id_header, '') AS message_id, \
                nullif(ae.storage_hash_sha256, '') AS storage_hash, \
                s.att_fp AS attachment_fp, \
                nullif(ae.duplicate_body_hash, '') AS body_hash, \
                CASE \
                    WHEN ae.sender_email IS NOT NULL AND ae.sender_email <> '' \
                        AND ae.duplicate_recipient_fingerprint IS NOT NULL \
                    THEN lower(coalesce(sender_email, '')) || '|' || coalesce(duplicate_recipient_fingerprint, '') || '|' || CAST(sent_at AS TEXT) \
                END AS headers_fp \
            FROM archived_emails ae \
            LEFT JOIN attachment_sets s ON s.email_id = ae.id \
            WHERE ae.deleted_at IS NULL",
        )
        .unwrap();
    let signal_rows: Vec<SignalRow> = stmt
        .query_map([], |row| {
            Ok(SignalRow {
                id: row.get("id")?,
                values: [
                    row.get("storage_hash")?,
                    row.get("message_id")?,
                    row.get("headers_fp")?,
                    row.get("attachment_fp")?,
                    row.get("body_hash")?,
                ],
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();

    // value → member indexes, per signal (used for union + reason detection).
    let mut by_key_value: [HashMap<String, Vec<usize>>; 5] = Default::default();
    for (idx, row) in signal_rows.iter().enumerate() {
        for (k, value) in row.values.iter().enumerate() {
            if let Some(value) = value {
                by_key_value[k].entry(value.clone()).or_default().push(idx);
            }
        }
    }

    // Union-find over row indexes.
    let mut parent: Vec<usize> = (0..signal_rows.len()).collect();
    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = x;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }
    // Only STRONG signals link a cluster; weak signals are badge-only.
    for (k, key) in REASON_KEYS.iter().enumerate() {
        if !key.1 {
            continue;
        }
        for members in by_key_value[k].values() {
            for i in 1..members.len() {
                let ra = find(&mut parent, members[0]);
                let rb = find(&mut parent, members[i]);
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    // Assemble connected components in first-seen order (JS Map semantics).
    let mut component_order: Vec<usize> = Vec::new();
    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for idx in 0..signal_rows.len() {
        let root = find(&mut parent, idx);
        let entry = components.entry(root).or_insert_with(|| {
            component_order.push(root);
            Vec::new()
        });
        entry.push(idx);
    }

    // A cluster the user has ignored is keyed by its fingerprint (the min id).
    let ignored: HashSet<String> = load_ignored_fingerprints(conn);

    struct Cluster {
        ids: Vec<String>,
        min_id: String,
        reasons: Vec<&'static str>,
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    for root in &component_order {
        let member_idxs = &components[root];
        if member_idxs.len() < 2 {
            continue;
        }
        let ids: Vec<String> = member_idxs.iter().map(|i| signal_rows[*i].id.clone()).collect();
        let min_id = ids.iter().min().cloned().unwrap_or_default();
        if ignored.contains(&min_id) {
            continue;
        }
        let mut reasons: Vec<&'static str> = Vec::new();
        for (k, key) in REASON_KEYS.iter().enumerate() {
            let applies = by_key_value[k]
                .values()
                .any(|members| members.iter().filter(|m| member_idxs.contains(m)).count() >= 2);
            if applies {
                reasons.push(key.0);
            }
        }
        clusters.push(Cluster { ids, min_id, reasons });
    }

    // Per-badge group counts (over every non-ignored cluster, independent of the
    // active reason filter) plus an "all" total, so each filter pill can show its
    // own count regardless of which one is selected.
    let mut reason_counts: Map<String, Value> = Map::new();
    reason_counts.insert("all".into(), json!(clusters.len()));
    for key in REASON_KEYS.iter() {
        let n = clusters.iter().filter(|c| c.reasons.contains(&key.0)).count();
        reason_counts.insert(key.0.to_string(), json!(n));
    }

    let mut filtered: Vec<&Cluster> = clusters
        .iter()
        .filter(|c| reason.map_or(true, |r| c.reasons.contains(&r)))
        .collect();
    // count desc, then min id asc — total order, so stability is moot.
    filtered.sort_by(|a, b| {
        b.ids
            .len()
            .cmp(&a.ids.len())
            .then_with(|| a.min_id.cmp(&b.min_id))
    });

    let total_groups = filtered.len();
    let page_clusters = filtered
        .into_iter()
        .skip(offset)
        .take(normalized_limit as usize);

    let mut groups: Vec<Value> = Vec::new();
    for cluster in page_clusters {
        let emails = find_emails_by_ids(conn, &cluster.ids);
        let keeper = emails
            .first()
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if emails.len() <= 1 || keeper.is_empty() {
            continue; // groups.filter(...) in Node
        }
        let primary = REASON_KEYS
            .iter()
            .map(|key| key.0)
            .find(|r| cluster.reasons.contains(r))
            .or_else(|| cluster.reasons.first().copied());
        groups.push(json!({
            "groupKey": format!("cluster:{}", cluster.min_id),
            "reason": primary,
            "reasons": cluster.reasons,
            "fingerprint": cluster.min_id,
            "count": emails.len(),
            "keeperEmailId": keeper,
            "emails": emails,
        }));
    }

    json!({
        "groups": groups,
        "totalGroups": total_groups,
        "reasonCounts": reason_counts,
        "page": normalized_page,
        "limit": normalized_limit,
    })
}

/// Fingerprints (cluster min-ids) the user has chosen to ignore.
fn load_ignored_fingerprints(conn: &Connection) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT fingerprint FROM exact_duplicate_ignores") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for fp in rows.flatten() {
                set.insert(fp);
            }
        }
    }
    set
}
