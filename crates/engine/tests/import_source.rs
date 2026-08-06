mod common;
use common::*;
use std::io::Write;

fn write_eml_zip(path: &std::path::Path, messages: &[(&str, &str, &str)]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (entry, message_id, subject) in messages {
        zip.start_file(entry, options).unwrap();
        write!(
            zip,
            "Message-ID: {message_id}\nFrom: archive@example.com\nSubject: {subject}\n\nbody"
        )
        .unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn prepared_mbox_streams_normalized_email_and_progress() {
    let archive = TempArchive::new();
    let input = archive.dir.join("Nested Mail.mbox");
    let body = simple_mbox("Prepared mbox", "interface tracer");
    std::fs::write(&input, body.as_bytes()).unwrap();

    let prepared = pea_engine::import_source::prepare(
        "mbox_import",
        &json!({ "localFilePath": input.to_string_lossy() }),
    )
    .unwrap();
    let mut subjects = Vec::new();
    let mut processed_bytes = 0;
    prepared
        .stream(&mut |email| subjects.push(email.subject), &mut |delta| {
            processed_bytes += delta
        })
        .unwrap();

    assert_eq!(prepared.display_name(), "Nested Mail");
    assert_eq!(prepared.total_bytes(), body.len() as u64);
    assert_eq!(subjects, ["Prepared mbox"]);
    assert_eq!(processed_bytes, body.len() as u64);
}

#[test]
fn prepared_eml_zip_filters_entries_and_completes_progress() {
    let archive = TempArchive::new();
    let input = archive.dir.join("Mail Export.zip");
    let file = std::fs::File::create(&input).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in [
        (
            "accounts/one.eml",
            "Message-ID: <zip-one@x>\nFrom: a@x\nSubject: Zip one\n\nbody",
        ),
        (
            "two.eml",
            "Message-ID: <zip-two@x>\nFrom: b@x\nSubject: Zip two\n\nbody",
        ),
        ("notes.txt", "not email"),
        ("__MACOSX/ghost.eml", "not email"),
        ("broken.eml", ""),
    ] {
        zip.start_file(name, options).unwrap();
        std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();

    let prepared = pea_engine::import_source::prepare(
        "eml_import",
        &json!({ "localFilePath": input.to_string_lossy() }),
    )
    .unwrap();
    let mut messages = Vec::new();
    let mut processed_bytes = 0;
    prepared
        .stream(
            &mut |email| messages.push((email.subject, email.path)),
            &mut |delta| processed_bytes += delta,
        )
        .unwrap();

    messages.sort();
    assert_eq!(prepared.display_name(), "Mail Export");
    assert_eq!(
        messages,
        [
            ("Zip one".into(), "accounts".into()),
            ("Zip two".into(), "".into())
        ]
    );
    assert_eq!(processed_bytes, prepared.total_bytes());
}

#[test]
fn mbox_adapter_owns_recursive_apple_and_loose_eml_discovery() {
    let archive = TempArchive::new();
    let root = archive.dir.join("selected");
    std::fs::create_dir_all(root.join("folders")).unwrap();
    std::fs::write(root.join("Inbox.mbox"), simple_mbox("Mbox message", "body")).unwrap();
    std::fs::write(
        root.join("folders/loose.eml"),
        "Message-ID: <loose@x>\nFrom: a@x\nSubject: Loose EML\n\nbody",
    )
    .unwrap();
    std::fs::write(root.join("folders/._ghost.eml"), "not email").unwrap();

    let emlx_dir = root.join("Archive.mbox/Data/1/Messages");
    std::fs::create_dir_all(&emlx_dir).unwrap();
    let emlx_message = "Message-ID: <emlx@x>\nFrom: a@x\nSubject: Apple EMLX\n\nbody";
    std::fs::write(
        emlx_dir.join("one.emlx"),
        format!("{}\n{}<plist/>", emlx_message.len(), emlx_message),
    )
    .unwrap();

    let export_dir = root.join("Sent.mbox");
    std::fs::create_dir_all(&export_dir).unwrap();
    std::fs::write(export_dir.join("mbox"), simple_mbox("Apple Export", "body")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&export_dir, root.join("loop.mbox")).unwrap();

    let prepared = pea_engine::import_source::prepare(
        "mbox_import",
        &json!({ "localFilePath": root.to_string_lossy() }),
    )
    .unwrap();
    let mut messages = Vec::new();
    let mut progress = Vec::new();
    let mut processed = 0;
    prepared
        .stream(
            &mut |email| messages.push((email.subject, email.path)),
            &mut |delta| {
                processed += delta;
                progress.push(processed);
            },
        )
        .unwrap();

    messages.sort();
    assert_eq!(
        messages,
        [
            ("Apple EMLX".into(), "Archive".into()),
            ("Apple Export".into(), "Sent".into()),
            ("Loose EML".into(), "folders".into()),
            ("Mbox message".into(), "Inbox".into()),
        ]
    );
    assert!(progress.windows(2).all(|values| values[0] <= values[1]));
    assert_eq!(processed, prepared.total_bytes());
}

#[test]
fn preparation_rejects_invalid_configuration_through_one_interface() {
    let archive = TempArchive::new();
    let empty = archive.dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let bad_zip = archive.dir.join("broken.zip");
    std::fs::write(&bad_zip, "not a zip").unwrap();

    for (provider, config, expected) in [
        ("mbox_import", json!({}), "path not provided"),
        (
            "mbox_import",
            json!({ "localFilePath": archive.dir.join("missing.mbox").to_string_lossy() }),
            "not found",
        ),
        (
            "mbox_import",
            json!({ "localFilePath": empty.to_string_lossy() }),
            "No mbox",
        ),
        (
            "eml_import",
            json!({ "localFilePath": archive.dir.join("mail.txt").to_string_lossy() }),
            "ZIP format",
        ),
        (
            "eml_import",
            json!({ "localFilePath": bad_zip.to_string_lossy() }),
            "Invalid EML Zip",
        ),
        ("future_import", json!({}), "Unsupported provider"),
    ] {
        let error = pea_engine::import_source::prepare(provider, &config)
            .err()
            .expect("configuration should fail");
        assert!(error.contains(expected), "{provider}: {error}");
    }
}

#[test]
fn terminal_failure_reports_the_selected_input() {
    let archive = TempArchive::new();
    let input = archive.dir.join("Disappearing.mbox");
    std::fs::write(&input, simple_mbox("Temporary", "body")).unwrap();
    let state = archive.state(false);
    let conn = state.pool.get().unwrap();
    let source_id = pea_engine::sources::create_source(
        &state,
        &conn,
        &json!({
            "provider": "mbox_import",
            "providerConfig": { "localFilePath": input.to_string_lossy() },
        }),
    )
    .unwrap();
    drop(conn);
    std::fs::remove_file(&input).unwrap();
    pea_engine::queue::drain_for_cli(&state).unwrap();

    let conn = state.pool.get().unwrap();
    let (status, message): (String, String) = conn
        .query_row(
            "SELECT status, last_import_status_message FROM ingestion_sources WHERE id = ?",
            [&source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "error");
    assert!(message.contains("Disappearing.mbox"), "{message}");
}

#[test]
fn reimport_failure_is_terminal_and_reports_the_selected_input() {
    let archive = TempArchive::new();
    let input = archive.dir.join("Reimport Failure.mbox");
    std::fs::write(&input, simple_mbox("First pass", "body")).unwrap();
    let state = archive.state(false);
    let conn = state.pool.get().unwrap();
    let source_id = pea_engine::sources::create_source(
        &state,
        &conn,
        &json!({
            "provider": "mbox_import",
            "providerConfig": { "localFilePath": input.to_string_lossy() },
        }),
    )
    .unwrap();
    drop(conn);
    pea_engine::queue::drain_for_cli(&state).unwrap();

    let conn = state.pool.get().unwrap();
    pea_engine::sources::trigger_reimport(&state, &conn, &source_id).unwrap();
    drop(conn);
    std::fs::remove_file(&input).unwrap();
    pea_engine::queue::drain_for_cli(&state).unwrap();

    let conn = state.pool.get().unwrap();
    let (status, message): (String, String) = conn
        .query_row(
            "SELECT status, last_import_status_message FROM ingestion_sources WHERE id = ?",
            [&source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "error");
    assert!(message.contains("Reimport Failure.mbox"), "{message}");
}

#[test]
fn queued_import_uses_one_preparation_snapshot_for_progress() {
    let archive = TempArchive::new();
    let input = archive.dir.join("changing-folder");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(
        input.join("one.eml"),
        "Message-ID: <snapshot-one@x>\nFrom: a@x\nSubject: One\n\nbody",
    )
    .unwrap();

    let state = archive.state(false);
    let conn = state.pool.get().unwrap();
    let source_id = pea_engine::sources::create_source(
        &state,
        &conn,
        &json!({
            "provider": "mbox_import",
            "providerConfig": { "localFilePath": input.to_string_lossy() },
        }),
    )
    .unwrap();
    pea_engine::queue::remove_jobs_by_source_id(&conn, &source_id);
    pea_engine::processors::dispatch(
        &state,
        "ingestion",
        "initial-import",
        &json!({ "ingestionSourceId": source_id }),
    )
    .unwrap();

    std::fs::write(
        input.join("two.eml"),
        "Message-ID: <snapshot-two@x>\nFrom: a@x\nSubject: Two\n\nlarger body",
    )
    .unwrap();
    let payload: String = conn
        .query_row(
            "SELECT payload FROM jobs WHERE name = 'process-mailbox' AND state = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    pea_engine::processors::dispatch(
        &state,
        "ingestion",
        "process-mailbox",
        &serde_json::from_str(&payload).unwrap(),
    )
    .unwrap();

    let progress = pea_engine::sources::import_progress_json(&conn, &source_id);
    assert_eq!(progress["processedBytes"], progress["totalBytes"]);
    assert_eq!(progress["percent"], 100);
}

#[test]
fn eml_zip_initial_import_and_reimport_share_the_full_cycle() {
    let archive = TempArchive::new();
    let input = archive.dir.join("Operations.zip");
    write_eml_zip(&input, &[("team/one.eml", "<cycle-one@x>", "Cycle one")]);

    let state = archive.state(false);
    let conn = state.pool.get().unwrap();
    let source_id = pea_engine::sources::create_source(
        &state,
        &conn,
        &json!({
            "provider": "eml_import",
            "providerConfig": { "localFilePath": input.to_string_lossy() },
        }),
    )
    .unwrap();
    drop(conn);
    pea_engine::queue::drain_for_cli(&state).unwrap();

    let conn = state.pool.get().unwrap();
    let first: (i64, i64, String, String, String) = conn
        .query_row(
            "SELECT count(*), count(email_fts.rowid), ingestion_sources.status, \
                    archived_emails.import_source, archived_emails.source_path \
             FROM archived_emails \
             JOIN ingestion_sources ON ingestion_sources.id = archived_emails.ingestion_source_id \
             LEFT JOIN email_fts ON email_fts.rowid = archived_emails.rowid",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        first,
        (1, 1, "imported".into(), "Operations".into(), "team".into())
    );

    write_eml_zip(
        &input,
        &[
            ("team/one.eml", "<cycle-one@x>", "Cycle one"),
            ("team/two.eml", "<cycle-two@x>", "Cycle two"),
        ],
    );
    pea_engine::sources::trigger_reimport(&state, &conn, &source_id).unwrap();
    drop(conn);
    pea_engine::queue::drain_for_cli(&state).unwrap();

    let conn = state.pool.get().unwrap();
    let after: (i64, i64, String) = conn
        .query_row(
            "SELECT count(*), count(email_fts.rowid), ingestion_sources.status \
             FROM archived_emails \
             JOIN ingestion_sources ON ingestion_sources.id = archived_emails.ingestion_source_id \
             LEFT JOIN email_fts ON email_fts.rowid = archived_emails.rowid",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(after, (2, 2, "imported".into()));
}
