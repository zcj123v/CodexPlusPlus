use codex_plus_core::codex_sqlite::{
    count_windows_extended_thread_cwds, relative_to_codex_home, sanitize_historical_model_suffixes,
    sanitize_windows_extended_thread_cwds,
};
use rusqlite::Connection;
use std::path::PathBuf;

fn create_threads_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model TEXT,
            updated_at INTEGER
        )",
        [],
    )
    .unwrap();
}

#[test]
fn sanitize_windows_extended_thread_cwds_repairs_all_session_databases() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    std::fs::create_dir_all(&sqlite_dir).unwrap();

    let root_db = home.join("state_5.sqlite");
    let root = Connection::open(&root_db).unwrap();
    root.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, updated_at INTEGER)",
        [],
    )
    .unwrap();
    root.execute(
        "INSERT INTO threads VALUES ('drive', ?1, 300)",
        [r"\\?\D:\MCAgentPlugin"],
    )
    .unwrap();
    root.execute(
        "INSERT INTO threads VALUES ('plain', ?1, 200)",
        [r"D:\Workspace\AIGC_Detect"],
    )
    .unwrap();
    root.execute(
        "INSERT INTO threads VALUES ('device', ?1, 150)",
        [r"\\?\Volume{1234}\project"],
    )
    .unwrap();
    root.execute(
        "INSERT INTO threads VALUES ('malformed', ?1, 125)",
        [r"\\?\"],
    )
    .unwrap();
    drop(root);

    let nested_db = sqlite_dir.join("state_5.sqlite");
    let nested = Connection::open(&nested_db).unwrap();
    nested
        .execute(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, updated_at INTEGER)",
            [],
        )
        .unwrap();
    nested
        .execute(
            "INSERT INTO threads VALUES ('unc', ?1, 100)",
            [r"\\?\UNC\server\share\project"],
        )
        .unwrap();
    nested
        .execute(
            "INSERT INTO threads VALUES ('unc-server-only', ?1, 90)",
            [r"\\?\UNC\server"],
        )
        .unwrap();
    drop(nested);

    assert_eq!(count_windows_extended_thread_cwds(&home).unwrap(), 2);
    let result = sanitize_windows_extended_thread_cwds(&home).unwrap();

    assert_eq!(result.scanned, 5);
    assert_eq!(result.updated, 2);
    let root = Connection::open(root_db).unwrap();
    assert_eq!(
        root.query_row("SELECT cwd FROM threads WHERE id = 'drive'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        r"D:\MCAgentPlugin"
    );
    assert_eq!(
        root.query_row("SELECT cwd FROM threads WHERE id = 'plain'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        r"D:\Workspace\AIGC_Detect"
    );
    assert_eq!(
        root.query_row("SELECT cwd FROM threads WHERE id = 'device'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        r"\\?\Volume{1234}\project"
    );
    assert_eq!(
        root.query_row(
            "SELECT cwd FROM threads WHERE id = 'malformed'",
            [],
            |row| { row.get::<_, String>(0) }
        )
        .unwrap(),
        r"\\?\"
    );
    let nested = Connection::open(nested_db).unwrap();
    assert_eq!(
        nested
            .query_row("SELECT cwd FROM threads WHERE id = 'unc'", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        r"\\server\share\project"
    );
    assert_eq!(
        nested
            .query_row(
                "SELECT cwd FROM threads WHERE id = 'unc-server-only'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        r"\\?\UNC\server"
    );
}

#[test]
fn external_sqlite_backup_paths_are_stable_and_do_not_collide() {
    let home = PathBuf::from("C:/Users/test/.codex");
    let spaced = PathBuf::from("D:/A B/state_5.sqlite");
    let underscored = PathBuf::from("D:/A_B/state_5.sqlite");

    let first = relative_to_codex_home(&home, &spaced);

    assert_eq!(first, relative_to_codex_home(&home, &spaced));
    assert_ne!(first, relative_to_codex_home(&home, &underscored));
    assert!(first.starts_with("external"));
    assert_eq!(first.file_name().unwrap(), "state_5.sqlite");
}

#[test]
fn sanitize_historical_model_suffixes_does_not_rewrite_thread_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model TEXT,
            cwd TEXT,
            updated_at INTEGER
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads VALUES ('t1', 'gpt-5.5', ?1, 1000)",
        [r"\\?\D:\MCAgentPlugin"],
    )
    .unwrap();
    drop(conn);

    sanitize_historical_model_suffixes(&home).unwrap();

    let conn = Connection::open(db_path).unwrap();
    let cwd: String = conn
        .query_row("SELECT cwd FROM threads WHERE id = 't1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(cwd, r"\\?\D:\MCAgentPlugin");
}

#[test]
fn sanitize_strips_suffix_from_thread_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        ["t1", "deepseek/deepseek-v4-flash[1M]", "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 1);
    assert_eq!(result.updated, 1);

    let conn = Connection::open(&db_path).unwrap();
    let model: String = conn
        .query_row("SELECT model FROM threads WHERE id = 't1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(model, "deepseek/deepseek-v4-flash");
}

#[test]
fn sanitize_skips_models_without_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        ["t1", "gpt-5.5", "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);
}

#[test]
fn sanitize_skips_invalid_suffixes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        ["t1", "foo[bar]", "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 1);
    assert_eq!(result.updated, 0);
}

#[test]
fn sanitize_handles_null_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        rusqlite::params!["t1", rusqlite::types::Null, "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);
}

#[test]
fn sanitize_cleans_suffix_from_logs() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();

    // logs_2.sqlite 不需要 threads 表，只需要 logs 表。
    let logs_path = home.join("logs_2.sqlite");
    let conn = Connection::open(&logs_path).unwrap();
    conn.execute(
        "CREATE TABLE logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            ts_nanos INTEGER NOT NULL,
            level TEXT NOT NULL,
            target TEXT NOT NULL,
            feedback_log_body TEXT,
            module_path TEXT,
            file TEXT,
            line INTEGER,
            thread_id TEXT,
            process_uuid TEXT,
            estimated_bytes INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        [
            "1",
            "1",
            "INFO",
            "codex_models_manager::cache",
            r#"session_loop{model="deepseek-v4-flash[1M]"}: Unknown model deepseek-v4-flash[1M] is used."#,
        ],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    // threads 表为空，所以 scanned/updated 都是 0；但日志应被清理。
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);

    let conn = Connection::open(&logs_path).unwrap();
    let body: String = conn
        .query_row(
            "SELECT feedback_log_body FROM logs WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !body.contains("[1M]"),
        "expected suffix to be stripped from logs, got: {body}"
    );
    assert!(body.contains("deepseek-v4-flash"));
}
