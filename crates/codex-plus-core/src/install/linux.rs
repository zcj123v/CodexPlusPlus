use std::path::{Path, PathBuf};

use super::{
    InstallOptions, MANAGER_BINARY, MANAGER_NAME, SILENT_BINARY, SILENT_NAME,
    install_root_or_default, option_or_current_exe,
};

/// 桌面文件 ID 与发行版包（如 Arch 包）保持一致，避免同一应用出现两份入口。
pub(crate) const SILENT_DESKTOP_FILE: &str = "codex-plus-plus.desktop";
pub(crate) const MANAGER_DESKTOP_FILE: &str = "codex-plus-plus-manager.desktop";

/// 系统级 applications 目录：发行版包管理器安装的入口位置（只检测，不写入）。
const SYSTEM_APPLICATIONS_DIRS: &[&str] =
    &["/usr/local/share/applications", "/usr/share/applications"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxDesktopEntry {
    pub file_path: PathBuf,
    pub contents: String,
}

pub fn build_desktop_entries(options: &InstallOptions) -> Vec<LinuxDesktopEntry> {
    let install_root = install_root_or_default(options);
    let launcher = option_or_current_exe(&options.launcher_path, SILENT_BINARY);
    let manager = option_or_current_exe(&options.manager_path, MANAGER_BINARY);
    vec![
        LinuxDesktopEntry {
            file_path: install_root.join(SILENT_DESKTOP_FILE),
            contents: desktop_entry(
                SILENT_NAME,
                "Codex Launcher",
                "Launch Codex with Codex++ enhancements",
                &launcher,
                "Development;IDE;",
            ),
        },
        LinuxDesktopEntry {
            file_path: install_root.join(MANAGER_DESKTOP_FILE),
            contents: desktop_entry(
                MANAGER_NAME,
                "Codex++ Manager",
                "Manage Codex++ enhancements, providers and user scripts",
                &manager,
                "Development;Settings;",
            ),
        },
    ]
}

/// 检测候选路径：优先用户级安装根（我们写入的位置），其次是系统级目录
/// （发行版包安装的位置）。
pub(crate) fn desktop_entry_candidates(root: &Option<PathBuf>, manager: bool) -> Vec<PathBuf> {
    let file = if manager {
        MANAGER_DESKTOP_FILE
    } else {
        SILENT_DESKTOP_FILE
    };
    let mut candidates = Vec::new();
    if let Some(root) = root {
        candidates.push(root.join(file));
    }
    for system_dir in SYSTEM_APPLICATIONS_DIRS {
        let candidate = PathBuf::from(system_dir).join(file);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn desktop_entry(
    name: &str,
    generic: &str,
    comment: &str,
    exec: &Path,
    categories: &str,
) -> String {
    format!(
        concat!(
            "[Desktop Entry]\n",
            "Type=Application\n",
            "Version=1.0\n",
            "Name={name}\n",
            "GenericName={generic}\n",
            "Comment={comment}\n",
            "Exec={exec}\n",
            "Icon={icon}\n",
            "Terminal=false\n",
            "Categories={categories}\n",
            "Keywords=codex;ai;\n",
            "StartupNotify=true\n",
        ),
        name = name,
        generic = generic,
        comment = comment,
        exec = desktop_entry_exec(exec),
        icon = desktop_entry_icon(exec),
        categories = categories,
    )
}

/// freedesktop Exec 字段中含空格的路径需要用双引号包裹。
fn desktop_entry_exec(exec: &Path) -> String {
    let path = exec.to_string_lossy();
    if path.chars().any(char::is_whitespace) {
        format!("\"{path}\"")
    } else {
        path.into_owned()
    }
}

/// 发行版包会注册 `codexplusplus` 主题图标；便携版可以把 png 放在
/// 二进制旁边，此时使用绝对路径作为图标。
fn desktop_entry_icon(exec: &Path) -> String {
    for name in ["codexplusplus.png", "codex-plus-plus.png"] {
        let sibling = exec.parent().unwrap_or_else(|| Path::new(".")).join(name);
        if sibling.exists() {
            return sibling.to_string_lossy().into_owned();
        }
    }
    "codexplusplus".to_string()
}

#[cfg(target_os = "linux")]
pub fn install_desktop_entries(options: &InstallOptions) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    for entry in build_desktop_entries(options) {
        crate::settings::atomic_write(&entry.file_path, entry.contents.as_bytes())?;
        // 桌面环境只把带可执行位的桌面快捷方式视为可启动。
        let mut permissions = std::fs::metadata(&entry.file_path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&entry.file_path, permissions)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall_desktop_entries(options: &InstallOptions) -> anyhow::Result<()> {
    // 只删除用户级安装根里的入口；系统级目录属于发行版包管理器，不动。
    let install_root = install_root_or_default(options);
    for file in [SILENT_DESKTOP_FILE, MANAGER_DESKTOP_FILE] {
        let file = install_root.join(file);
        if file.exists() {
            std::fs::remove_file(file)?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn install_desktop_entries(_options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("Linux desktop entries are only supported on Linux")
}

#[cfg(not(target_os = "linux"))]
pub fn uninstall_desktop_entries(_options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("Linux desktop entries are only supported on Linux")
}
