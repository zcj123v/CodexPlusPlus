use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_UPDATE_REPOSITORY: &str = "zcj123v/CodexPlusPlus";
pub const DEFAULT_LATEST_JSON_URL: &str =
    "https://github.com/zcj123v/CodexPlusPlus/releases/latest/download/latest.json";

pub const DEFAULT_REPOSITORY: &str = DEFAULT_UPDATE_REPOSITORY;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub body: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_summary: String,
    pub release_url: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInstall {
    pub release: Release,
    pub installer_path: PathBuf,
    pub launched: bool,
}

pub fn parse_version_tag(value: &str) -> anyhow::Result<Vec<u64>> {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    let mut digits = String::new();
    for ch in normalized.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            digits.push(ch);
        } else {
            break;
        }
    }
    if digits.is_empty() {
        anyhow::bail!("Invalid version tag: {value}");
    }
    digits
        .split('.')
        .map(|part| part.parse::<u64>().map_err(Into::into))
        .collect()
}

fn linux_revision(value: &str) -> Option<u64> {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    let (_, suffix) = normalized.split_once("-linux.")?;
    (!suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

pub fn is_newer_version(candidate: &str, current: &str) -> anyhow::Result<bool> {
    let mut left = parse_version_tag(candidate)?;
    let mut right = parse_version_tag(current)?;
    let len = left.len().max(right.len());
    left.resize(len, 0);
    right.resize(len, 0);
    if left != right {
        return Ok(left > right);
    }
    Ok(linux_revision(candidate).unwrap_or(0) > linux_revision(current).unwrap_or(0))
}

pub fn release_from_github_payload(payload: &Value) -> anyhow::Result<Release> {
    let version = payload
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("release payload missing tag_name"))?
        .to_string();
    let assets = payload
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            Some((
                asset.get("name")?.as_str()?.to_string(),
                asset.get("browser_download_url")?.as_str()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    let selected = select_update_asset(&assets);
    Ok(Release {
        version,
        url: payload
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: payload
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        asset_name: selected.as_ref().map(|asset| asset.name.clone()),
        asset_url: selected.map(|asset| asset.browser_download_url),
    })
}

pub fn release_from_latest_json_payload(payload: &Value) -> anyhow::Result<Release> {
    release_from_latest_json_payload_for_platform(payload, UpdatePlatform::current())
}

pub fn release_from_latest_json_payload_for_platform(
    payload: &Value,
    platform: UpdatePlatform,
) -> anyhow::Result<Release> {
    let version = payload
        .get("version")
        .or_else(|| payload.get("tag_name"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("latest.json missing version"))?
        .to_string();
    let assets = payload
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset.get("name")?.as_str()?.to_string();
            let url = asset
                .get("url")
                .or_else(|| asset.get("browser_download_url"))?
                .as_str()?
                .to_string();
            Some((name, url))
        })
        .collect::<Vec<_>>();
    let selected = select_update_asset_for_platform(&assets, platform);
    Ok(Release {
        version,
        url: payload
            .get("url")
            .or_else(|| payload.get("html_url"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: payload
            .get("body")
            .or_else(|| payload.get("release_summary"))
            .or_else(|| payload.get("notes"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        asset_name: selected.as_ref().map(|asset| asset.name.clone()),
        asset_url: selected.map(|asset| asset.browser_download_url),
    })
}

pub fn select_update_asset(assets: &[(String, String)]) -> Option<ReleaseAsset> {
    select_update_asset_for_platform(assets, UpdatePlatform::current())
}

pub fn select_update_asset_for_platform(
    assets: &[(String, String)],
    platform: UpdatePlatform,
) -> Option<ReleaseAsset> {
    let named = assets
        .iter()
        .filter(|(name, url)| !name.trim().is_empty() && !url.trim().is_empty());
    let mut best: Option<(u8, &str, &str)> = None;
    for (name, url) in named {
        let Some(rank) = platform_asset_rank(&name.to_ascii_lowercase(), platform) else {
            continue;
        };
        if best.map_or(true, |(r, _, _)| rank < r) {
            best = Some((rank, name.as_str(), url.as_str()));
        }
    }
    best.map(|(_, name, url)| ReleaseAsset {
        name: name.to_string(),
        browser_download_url: url.to_string(),
    })
}

pub async fn fetch_latest_release(latest_json_url: &str) -> anyhow::Result<Release> {
    let client =
        crate::http_client::proxied_client(&format!("Codex++/{}", crate::version::VERSION))?;
    let payload = client
        .get(latest_json_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    release_from_latest_json_payload(&payload)
}

pub async fn check_for_update(current_version: &str) -> anyhow::Result<UpdateCheck> {
    let release = fetch_latest_release(DEFAULT_LATEST_JSON_URL).await?;
    let update_available = is_newer_version(&release.version, current_version)?;
    Ok(UpdateCheck {
        current_version: current_version.to_string(),
        latest_version: Some(release.version),
        release_summary: release.body,
        release_url: release.url,
        asset_name: release.asset_name,
        asset_url: release.asset_url,
        update_available,
    })
}

pub async fn perform_update(
    release: &Release,
    download_dir: &Path,
) -> anyhow::Result<UpdateInstall> {
    let url = release
        .asset_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("没有可下载的 Release asset"))?;
    let bytes =
        crate::http_client::proxied_client(&format!("Codex++/{}", crate::version::VERSION))?
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
    let installer_path = download_asset_to(release, &bytes, download_dir)?;
    launch_installer(&installer_path)?;
    Ok(UpdateInstall {
        release: release.clone(),
        installer_path,
        launched: true,
    })
}

pub fn download_asset_to(
    release: &Release,
    bytes: &[u8],
    download_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let name = release
        .asset_name
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("没有可下载的 Release asset"))?;
    let safe = safe_asset_name(name)?;
    std::fs::create_dir_all(download_dir)?;
    let path = download_dir.join(safe);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

pub fn safe_asset_name(name: &str) -> anyhow::Result<String> {
    if name.trim().is_empty() {
        anyhow::bail!("非法 Release asset 文件名: {name}");
    }
    let path = Path::new(name);
    if path.components().count() != 1 {
        anyhow::bail!("非法 Release asset 文件名: {name}");
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("非法 Release asset 文件名: {name}"))?;
    if file_name == "." || file_name == ".." {
        anyhow::bail!("非法 Release asset 文件名: {name}");
    }
    Ok(file_name.to_string())
}

pub fn classify_linux_os_release(contents: &str) -> LinuxPackageFamily {
    let mut id = "";
    let mut id_like = "";
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(|ch| ch == '\'' || ch == '"');
        match key.trim() {
            "ID" => id = value,
            "ID_LIKE" => id_like = value,
            _ => {}
        }
    }
    for identifier in std::iter::once(id).chain(id_like.split_ascii_whitespace()) {
        match identifier.to_ascii_lowercase().as_str() {
            "arch" | "archlinux" | "cachyos" | "manjaro" | "endeavouros" => {
                return LinuxPackageFamily::Arch;
            }
            "debian" | "ubuntu" | "linuxmint" | "pop" => return LinuxPackageFamily::Debian,
            _ => {}
        }
    }
    LinuxPackageFamily::Unknown
}

fn read_linux_package_family(path: &Path) -> LinuxPackageFamily {
    std::fs::read_to_string(path)
        .map(|contents| classify_linux_os_release(&contents))
        .unwrap_or(LinuxPackageFamily::Unknown)
}

fn platform_asset_rank(name: &str, platform: UpdatePlatform) -> Option<u8> {
    // 0 = exact match (current OS + native arch)
    // 1 = same OS, other arch (acceptable fallback, e.g. x86_64 on arm64 or vice versa)
    // None = wrong platform
    if platform.os == UpdateOs::Macos {
        if !is_macos_installer_asset(name) {
            return None;
        }
        return Some(if is_native_arch_asset(name, platform.arch) {
            0
        } else {
            1
        });
    }
    if platform.os == UpdateOs::Windows && is_windows_installer_asset(name) {
        return Some(0);
    }
    if platform.os != UpdateOs::Linux || is_non_linux_asset(name) {
        return None;
    }
    let package_family = if name.ends_with(".pkg.tar.zst") {
        LinuxPackageFamily::Arch
    } else if name.ends_with(".deb") {
        LinuxPackageFamily::Debian
    } else {
        return None;
    };
    let family_rank = match platform.linux_family {
        LinuxPackageFamily::Unknown => 1,
        family if family == package_family => 0,
        _ => 1,
    };
    let arch_rank = if is_native_arch_asset(name, platform.arch) {
        0
    } else {
        2
    };
    Some(family_rank + arch_rank)
}

fn is_native_arch_asset(name: &str, arch: UpdateArch) -> bool {
    let native_tokens: &[&str] = match arch {
        UpdateArch::X86_64 => &["x64", "x86_64", "amd64"],
        UpdateArch::Aarch64 => &["arm64", "aarch64"],
        _ => return true, // unknown arch — accept anything
    };
    let all_tokens = ["x64", "x86_64", "amd64", "arm64", "aarch64"];
    let mentioned = |token: &str| {
        name.contains(&format!("-{token}."))
            || name.contains(&format!("_{token}."))
            || name.contains(&format!("-{token}-"))
            || name.contains(&format!("_{token}-"))
    };
    native_tokens.iter().any(|token| mentioned(token))
        || !all_tokens.iter().any(|token| mentioned(token))
}

fn is_non_linux_asset(name: &str) -> bool {
    name.contains("debug")
        || name.contains("source")
        || name.ends_with(".zip")
        || name.ends_with(".dmg")
        || name.ends_with(".exe")
        || name.ends_with(".msi")
}

fn is_windows_installer_asset(name: &str) -> bool {
    name.contains("codex")
        && name.contains("plus")
        && (name.ends_with(".msi")
            || name.ends_with("-setup.exe")
            || name.ends_with("_setup.exe")
            || name.ends_with("setup.exe")
            || name.ends_with("installer.exe"))
}

fn is_macos_installer_asset(name: &str) -> bool {
    // Loose shape check; arch preference is handled by platform_asset_rank
    // via is_macos_native_arch_asset.
    name.contains("codex") && name.contains("plus") && name.ends_with(".dmg")
}

pub fn launch_installer(path: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(path)
            .creation_flags(crate::windows_integration::CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("启动安装包失败：{error}"))
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("打开 DMG 失败：{error}"))
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = path;
        anyhow::bail!("当前平台不支持启动安装包")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxPackageFamily {
    Arch,
    Debian,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOs {
    Windows,
    Macos,
    Linux,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateArch {
    X86_64,
    Aarch64,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdatePlatform {
    pub os: UpdateOs,
    pub arch: UpdateArch,
    pub linux_family: LinuxPackageFamily,
}

impl UpdatePlatform {
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "windows" => UpdateOs::Windows,
            "macos" => UpdateOs::Macos,
            "linux" => UpdateOs::Linux,
            _ => UpdateOs::Other,
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => UpdateArch::X86_64,
            "aarch64" => UpdateArch::Aarch64,
            _ => UpdateArch::Other,
        };
        let linux_family = if os == UpdateOs::Linux {
            read_linux_package_family(Path::new("/etc/os-release"))
        } else {
            LinuxPackageFamily::Unknown
        };
        Self {
            os,
            arch,
            linux_family,
        }
    }
}
