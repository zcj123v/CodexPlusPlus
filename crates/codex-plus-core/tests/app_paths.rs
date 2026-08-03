use std::path::Path;

fn write_test_asar(path: &Path, package_json: &str) {
    let header = format!(
        "{{\"files\":{{\"package.json\":{{\"size\":{},\"offset\":\"0\"}}}}}}",
        package_json.len()
    );
    let json_len = header.len() as u32;
    let padded = json_len.div_ceil(4) * 4;
    // 头部 pickle = payload 长度字段（4）+ 字符串长度字段（4）+ 对齐后的 JSON
    let header_pickle_size = 8 + padded;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&header_pickle_size.to_le_bytes());
    bytes.extend_from_slice(&padded.to_le_bytes());
    bytes.extend_from_slice(&json_len.to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    bytes.resize(8 + header_pickle_size as usize, 0);
    bytes.extend_from_slice(package_json.as_bytes());
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn codex_app_version_reads_version_from_linux_electron_asar() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("openai-codex-desktop");
    let resources = app_dir.join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    write_test_asar(
        &resources.join("app.asar"),
        r#"{"name":"openai-codex-electron","productName":"Codex","version":"26.707.31428"}"#,
    );

    assert_eq!(
        codex_plus_core::app_paths::codex_app_version(&app_dir),
        Some("26.707.31428".to_string())
    );
}

#[test]
fn codex_app_version_reads_version_from_unpacked_electron_app() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("codex-desktop");
    let app = app_dir.join("resources").join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("package.json"),
        r#"{"name":"codex","version":"1.2.3"}"#,
    )
    .unwrap();

    assert_eq!(
        codex_plus_core::app_paths::codex_app_version(&app_dir),
        Some("1.2.3".to_string())
    );
}

#[test]
fn codex_app_version_ignores_corrupt_asar() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("openai-codex-desktop");
    let resources = app_dir.join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(resources.join("app.asar"), b"not an asar archive").unwrap();

    assert_eq!(
        codex_plus_core::app_paths::codex_app_version(&app_dir),
        None
    );
}

// 社区 Linux 版（ilysenko/codex-desktop-linux）把 Electron 二进制命名为
// `electron`，旧 AUR/自建版的 `codex` 二进制不再受支持。
#[cfg(target_os = "linux")]
#[test]
fn normalize_codex_app_path_accepts_dir_with_electron_binary() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("codex-desktop");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("electron"), b"#!/bin/sh\n").unwrap();

    assert_eq!(
        codex_plus_core::app_paths::normalize_codex_app_path(&app_dir),
        Some(app_dir.clone())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn normalize_codex_app_path_rejects_dir_with_legacy_codex_binary() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("openai-codex-desktop");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("codex"), b"#!/bin/sh\n").unwrap();

    assert_eq!(
        codex_plus_core::app_paths::normalize_codex_app_path(&app_dir),
        None
    );
}
