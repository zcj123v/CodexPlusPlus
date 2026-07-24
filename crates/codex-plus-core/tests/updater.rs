use codex_plus_core::update::{
    DEFAULT_LATEST_JSON_URL, DEFAULT_UPDATE_REPOSITORY, LinuxPackageFamily, Release, UpdateArch,
    UpdateOs, UpdatePlatform, classify_linux_os_release, download_asset_to, is_newer_version,
    parse_version_tag, release_from_github_payload, release_from_latest_json_payload,
    release_from_latest_json_payload_for_platform, safe_asset_name, select_update_asset,
    select_update_asset_for_platform,
};
use serde_json::json;

fn linux(family: LinuxPackageFamily) -> UpdatePlatform {
    UpdatePlatform {
        os: UpdateOs::Linux,
        arch: UpdateArch::X86_64,
        linux_family: family,
    }
}

#[test]
fn classifies_linux_package_families() {
    assert_eq!(
        classify_linux_os_release("ID=cachyos\nID_LIKE=arch\n"),
        LinuxPackageFamily::Arch
    );
    assert_eq!(
        classify_linux_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"),
        LinuxPackageFamily::Debian
    );
    assert_eq!(
        classify_linux_os_release("NAME=Other\n"),
        LinuxPackageFamily::Unknown
    );
}

#[test]
fn linux_families_choose_native_non_debug_packages() {
    let assets = [
        "codexplusplus-debug-1.2.42-1-x86_64.pkg.tar.zst",
        "codexplusplus-1.2.42-1-x86_64.pkg.tar.zst",
        "codexplusplus_1.2.42_amd64.deb",
        "CodexPlusPlus-1.2.42-macos-x64.dmg",
    ]
    .into_iter()
    .map(|name| (name.to_string(), format!("https://example.test/{name}")))
    .collect::<Vec<_>>();
    assert_eq!(
        select_update_asset_for_platform(&assets, linux(LinuxPackageFamily::Arch))
            .unwrap()
            .name,
        "codexplusplus-1.2.42-1-x86_64.pkg.tar.zst"
    );
    assert_eq!(
        select_update_asset_for_platform(&assets, linux(LinuxPackageFamily::Debian))
            .unwrap()
            .name,
        "codexplusplus_1.2.42_amd64.deb"
    );
}

#[test]
fn source_only_manifest_preserves_release_url_without_selecting_asset() {
    let release_url = "https://github.com/zcj123v/CodexPlusPlus/releases/tag/v1.2.42-linux.1";
    let release = release_from_latest_json_payload_for_platform(
        &json!({
            "version": "v1.2.42-linux.1",
            "url": release_url,
            "assets": [
                {"name": "source.zip", "url": "https://example.test/source.zip"}
            ]
        }),
        linux(LinuxPackageFamily::Arch),
    )
    .unwrap();

    assert_eq!(release.url, release_url);
    assert_eq!(release.asset_name, None);
    assert_eq!(release.asset_url, None);
}
#[test]
fn update_source_points_to_fork() {
    assert_eq!(DEFAULT_UPDATE_REPOSITORY, "zcj123v/CodexPlusPlus");
    assert_eq!(
        DEFAULT_LATEST_JSON_URL,
        "https://github.com/zcj123v/CodexPlusPlus/releases/latest/download/latest.json"
    );
}

#[test]
fn fork_linux_revisions_sort_after_base_version() {
    assert!(is_newer_version("1.2.42-linux.1", "1.2.41").unwrap());
    assert!(is_newer_version("1.2.42-linux.1", "1.2.42").unwrap());
    assert!(is_newer_version("1.2.42-linux.2", "1.2.42-linux.1").unwrap());
    assert!(!is_newer_version("1.2.42-linux.1", "1.2.42-linux.1").unwrap());
    assert!(!is_newer_version("v1.2.42", "1.2.42").unwrap());
}

#[test]
fn parse_version_tag_accepts_prefix_and_suffix() {
    assert_eq!(parse_version_tag("v1.2.3").unwrap(), vec![1, 2, 3]);
    assert_eq!(parse_version_tag("1.2.3").unwrap(), vec![1, 2, 3]);
    assert_eq!(parse_version_tag("v1.2.3-beta.1").unwrap(), vec![1, 2, 3]);
}

#[test]
fn version_comparison_uses_numeric_segments() {
    assert!(is_newer_version("v1.0.10", "1.0.4").unwrap());
    assert!(!is_newer_version("v1.0.4", "1.0.4").unwrap());
    assert!(!is_newer_version("v1.0.3", "1.0.4").unwrap());
}

#[test]
fn github_payload_selects_platform_installer() {
    let release = release_from_github_payload(&json!({
        "tag_name": "v1.0.9",
        "html_url": "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.0.9",
        "body": "fixes",
        "assets": [
            {"name": "source.zip", "browser_download_url": "https://example.test/source.zip"},
            {"name": "codex-plus-plus-manager.exe", "browser_download_url": "https://example.test/manager.exe"},
            {"name": "CodexPlusPlus_1.0.9_x64-setup.exe", "browser_download_url": "https://example.test/setup.exe"},
            {"name": "CodexPlusPlus_1.0.9_x64.dmg", "browser_download_url": "https://example.test/app.dmg"}
        ]
    }))
    .unwrap();

    assert_eq!(release.version, "v1.0.9");
    if cfg!(windows) {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus_1.0.9_x64-setup.exe")
        );
    } else if cfg!(target_os = "macos") {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus_1.0.9_x64.dmg")
        );
    } else {
        assert_eq!(release.asset_name.as_deref(), None);
    }
}

#[test]
fn latest_json_payload_selects_platform_installer_without_github_api_shape() {
    let release = release_from_latest_json_payload(&json!({
        "version": "v1.1.6",
        "url": "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.1.6",
        "body": "静态更新描述",
        "assets": [
            {"name": "source.zip", "url": "https://example.test/source.zip"},
            {"name": "CodexPlusPlus-1.1.6-windows-x64-setup.exe", "url": "https://example.test/setup.exe"},
            {"name": "CodexPlusPlus-1.1.6-macos-x64.dmg", "url": "https://example.test/app.dmg"}
        ]
    }))
    .unwrap();

    assert_eq!(release.version, "v1.1.6");
    assert_eq!(release.body, "静态更新描述");
    if cfg!(windows) {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus-1.1.6-windows-x64-setup.exe")
        );
    } else if cfg!(target_os = "macos") {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus-1.1.6-macos-x64.dmg")
        );
    } else {
        assert_eq!(release.asset_name.as_deref(), None);
    }
}

#[test]
fn asset_selection_prefers_current_platform_artifacts() {
    let assets = vec![
        (
            "CodexPlusPlus.zip".to_string(),
            "https://example.test/source.zip".to_string(),
        ),
        (
            "codex-plus-plus-manager.exe".to_string(),
            "https://example.test/manager.exe".to_string(),
        ),
        (
            "CodexPlusPlus_1.0.9_x64-setup.exe".to_string(),
            "https://example.test/setup.exe".to_string(),
        ),
        (
            "CodexPlusPlus_1.0.9_x64.dmg".to_string(),
            "https://example.test/app.dmg".to_string(),
        ),
    ];

    if cfg!(windows) {
        let selected = select_update_asset(&assets).unwrap();
        assert_eq!(selected.name, "CodexPlusPlus_1.0.9_x64-setup.exe");
    } else if cfg!(target_os = "macos") {
        let selected = select_update_asset(&assets).unwrap();
        assert_eq!(selected.name, "CodexPlusPlus_1.0.9_x64.dmg");
    } else {
        assert!(select_update_asset(&assets).is_none());
    }
}

#[test]
fn asset_selection_distinguishes_x64_and_arm64_macos_dmgs() {
    // Regression test for the bug where an x86_64 Mac user could be handed
    // the arm64 DMG (or vice versa) because `is_macos_installer_asset` did
    // not check the arch token in the filename.
    let assets = vec![
        (
            "CodexPlusPlus-1.2.17-macos-arm64.dmg".to_string(),
            "https://example.test/app-arm64.dmg".to_string(),
        ),
        (
            "CodexPlusPlus-1.2.17-macos-x64.dmg".to_string(),
            "https://example.test/app-x64.dmg".to_string(),
        ),
    ];

    if cfg!(target_os = "macos") {
        let selected = select_update_asset(&assets)
            .expect("a macOS DMG should be selected for the running arch");
        let expected = match std::env::consts::ARCH {
            "x86_64" => "CodexPlusPlus-1.2.17-macos-x64.dmg",
            "aarch64" => "CodexPlusPlus-1.2.17-macos-arm64.dmg",
            other => panic!("unexpected target arch in test: {other}"),
        };
        assert_eq!(
            selected.name, expected,
            "x86_64 binary must select x64 DMG, aarch64 binary must select arm64 DMG"
        );
    } else {
        // Non-macOS platforms should not pick either macOS DMG.
        assert!(select_update_asset(&assets).is_none());
    }
}

#[test]
fn safe_asset_name_rejects_path_traversal() {
    assert_eq!(safe_asset_name("pkg.zip").unwrap(), "pkg.zip");
    assert!(safe_asset_name("../pkg.zip").is_err());
    assert!(safe_asset_name("").is_err());
}

#[test]
fn download_asset_to_writes_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let release = Release {
        version: "v1.0.9".to_string(),
        url: "https://example.test".to_string(),
        body: "fixes".to_string(),
        asset_name: Some("pkg.zip".to_string()),
        asset_url: Some("https://example.test/pkg.zip".to_string()),
    };

    let path = download_asset_to(&release, b"abcdef", dir.path()).unwrap();

    assert_eq!(path, dir.path().join("pkg.zip"));
    assert_eq!(std::fs::read(path).unwrap(), b"abcdef");
}
