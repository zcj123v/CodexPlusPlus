use std::fs::File;
use std::net::{TcpListener, ToSocketAddrs};
use std::path::{Path, PathBuf};

use fs2::FileExt;

pub const LAUNCHER_GUARD_PORT_BASE: u16 = 57320;
pub const MANAGER_GUARD_PORT_BASE: u16 = 57319;

/// Offset applied to guard port base to avoid conflicts in multi-user
/// environments (Windows RDP, shared servers, etc.).
///
/// Resolution order:
/// 1. `CODEX_PLUS_GUARD_PORT` env var — exact port override
/// 2. `CODEX_PLUS_GUARD_PORT_OFFSET` env var — explicit numeric offset
/// 3. Windows: hash of `USERNAME` (mod 1000) for per-user isolation
/// 4. Other platforms: 0 (backward-compatible default)
fn guard_port_offset() -> u16 {
    // env var exact port takes priority (caller handles it via override functions below)
    #[cfg(windows)]
    {
        if let Ok(user) = std::env::var("USERNAME") {
            let hash: u16 = user.bytes().fold(0u16, |acc, b| acc.wrapping_add(b as u16));
            return hash % 1000;
        }
    }
    0
}

/// Effective launcher guard port (base + auto-offset, overridable via env var).
pub fn launcher_guard_port() -> u16 {
    if let Some(port) = std::env::var("CODEX_PLUS_GUARD_PORT")
        .or_else(|_| std::env::var("CODEX_PLUS_LAUNCHER_GUARD_PORT"))
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
    {
        return port;
    }
    if let Some(offset) = std::env::var("CODEX_PLUS_GUARD_PORT_OFFSET")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
    {
        return LAUNCHER_GUARD_PORT_BASE + offset;
    }
    LAUNCHER_GUARD_PORT_BASE + guard_port_offset()
}

/// Effective manager guard port (base + auto-offset, overridable via env var).
pub fn manager_guard_port() -> u16 {
    if let Some(port) = std::env::var("CODEX_PLUS_GUARD_PORT")
        .or_else(|_| std::env::var("CODEX_PLUS_MANAGER_GUARD_PORT"))
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
    {
        return port;
    }
    if let Some(offset) = std::env::var("CODEX_PLUS_GUARD_PORT_OFFSET")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
    {
        return MANAGER_GUARD_PORT_BASE + offset;
    }
    MANAGER_GUARD_PORT_BASE + guard_port_offset()
}

pub fn select_platform_loopback_port(requested: u16) -> u16 {
    select_platform_loopback_port_with(
        requested,
        cfg!(windows),
        can_bind_loopback_port,
        find_available_loopback_port,
    )
}

pub fn select_packaged_codex_debug_port(requested: u16) -> u16 {
    select_packaged_codex_debug_port_with(
        requested,
        cfg!(windows),
        can_bind_loopback_port,
        crate::cdp::endpoint_available,
        find_available_loopback_port,
    )
}

pub fn select_packaged_codex_debug_port_with(
    requested: u16,
    is_windows: bool,
    can_bind: impl Fn(u16) -> bool,
    is_existing_cdp: impl Fn(u16) -> bool,
    find_available: impl Fn() -> u16,
) -> u16 {
    if !is_windows || can_bind(requested) || is_existing_cdp(requested) {
        requested
    } else {
        find_available()
    }
}

pub fn select_platform_loopback_port_with(
    requested: u16,
    is_windows: bool,
    can_bind: impl Fn(u16) -> bool,
    find_available: impl Fn() -> u16,
) -> u16 {
    if !is_windows || can_bind(requested) {
        requested
    } else {
        find_available()
    }
}

pub fn can_bind_loopback_port(port: u16) -> bool {
    if port == 0 {
        return true;
    }
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 找一个经「释放后可再次绑定」验证的 loopback 端口，找不到时返回 0
/// （保持原签名语义：调用方将 0 视为 bind ephemeral）。
pub fn find_available_loopback_port() -> u16 {
    find_rebindable_loopback_port().unwrap_or(0)
}

pub fn can_connect_loopback_port(port: u16) -> bool {
    ("127.0.0.1", port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .and_then(|address| {
            std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(200))
                .ok()
        })
        .is_some()
}

pub fn acquire_loopback_port_guard(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

#[derive(Debug)]
pub struct LoopbackPortGuard {
    _lock_file: Option<File>,
    lock_path: Option<PathBuf>,
    _listener: Option<TcpListener>,
    using_fallback_lock: bool,
}

impl LoopbackPortGuard {
    pub fn listener(listener: TcpListener) -> Self {
        Self {
            _lock_file: None,
            lock_path: None,
            _listener: Some(listener),
            using_fallback_lock: false,
        }
    }

    fn locked_listener(file: File, path: PathBuf, listener: TcpListener) -> Self {
        Self {
            _lock_file: Some(file),
            lock_path: Some(path),
            _listener: Some(listener),
            using_fallback_lock: false,
        }
    }

    fn fallback_lock(file: File, path: PathBuf) -> Self {
        Self {
            _lock_file: Some(file),
            lock_path: Some(path),
            _listener: None,
            using_fallback_lock: true,
        }
    }

    pub fn fallback_path(&self) -> Option<&Path> {
        self.using_fallback_lock
            .then_some(())
            .and_then(|_| self.lock_path.as_deref())
    }
}

pub fn acquire_resilient_loopback_port_guard(port: u16) -> std::io::Result<LoopbackPortGuard> {
    acquire_resilient_loopback_port_guard_at(port, &crate::paths::default_app_state_dir())
}

fn acquire_resilient_loopback_port_guard_at(
    port: u16,
    state_dir: &Path,
) -> std::io::Result<LoopbackPortGuard> {
    acquire_resilient_loopback_port_guard_with(
        port,
        state_dir,
        acquire_loopback_port_guard,
        can_connect_loopback_port,
    )
}

fn acquire_resilient_loopback_port_guard_with(
    port: u16,
    state_dir: &Path,
    bind: impl Fn(u16) -> std::io::Result<TcpListener>,
    can_connect: impl Fn(u16) -> bool,
) -> std::io::Result<LoopbackPortGuard> {
    if port == 0 {
        return bind(port).map(LoopbackPortGuard::listener);
    }

    let (file, path) = acquire_lock_guard(port, state_dir)?;
    match bind(port) {
        Ok(listener) => Ok(LoopbackPortGuard::locked_listener(file, path, listener)),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && can_connect(port) => {
            Err(error)
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::AddrInUse || port_bind_forbidden(&error) =>
        {
            Ok(LoopbackPortGuard::fallback_lock(file, path))
        }
        Err(error) => Err(error),
    }
}

fn port_bind_forbidden(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(10013))
}

fn acquire_lock_guard(port: u16, state_dir: &Path) -> std::io::Result<(File, PathBuf)> {
    let dir = state_dir.join("locks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("loopback-port-{port}.lock"));
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    file.try_lock_exclusive().map_err(normalize_lock_error)?;
    Ok((file, path))
}

fn normalize_lock_error(error: std::io::Error) -> std::io::Error {
    match error.raw_os_error() {
        Some(33) => std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "loopback port guard lock is already held",
        ),
        _ => error,
    }
}

/// Windows 下确定性候选端口数量：requested 及其后 7 个端口。
const PORT_FALLBACK_CANDIDATE_SPAN: u16 = 8;
/// `find_rebindable_loopback_port` 验证 ephemeral 端口可重绑的最大尝试次数。
const REBINDABLE_PORT_MAX_ATTEMPTS: usize = 3;

/// 判断错误是否为 Windows excluded port range 导致的拒绝绑定
/// （`WSAEACCES` / `os error 10013`，跨平台归一化为 `PermissionDenied`）。
/// 目前仅用于测试断言候选回退覆盖的错误类型，不作为公开 API。
#[cfg(test)]
fn is_excluded_port_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(10013) || error.kind() == std::io::ErrorKind::PermissionDenied
}

/// guard 获取结果：除 guard 本体外携带 requested/effective 端口与尝试次数，供日志记录。
#[derive(Debug)]
pub struct ResilientGuardAcquisition {
    pub guard: LoopbackPortGuard,
    pub requested_port: u16,
    pub effective_port: u16,
    pub attempts: usize,
}

/// 获取单实例 guard；Windows 下 requested 端口不可绑定（如落入 excluded port
/// range）时按确定性候选回退，最后尝试 ephemeral 端口。非 Windows 保持单次获取。
pub fn acquire_resilient_guard_with_port_fallback(
    requested_port: u16,
) -> std::io::Result<ResilientGuardAcquisition> {
    let state_dir = crate::paths::default_app_state_dir();
    acquire_resilient_guard_with_port_fallback_with(
        requested_port,
        cfg!(windows),
        &state_dir,
        acquire_resilient_loopback_port_guard_at,
        || acquire_loopback_port_guard(0),
    )
}

fn acquire_resilient_guard_with_port_fallback_with(
    requested_port: u16,
    is_windows: bool,
    state_dir: &Path,
    acquire: impl Fn(u16, &Path) -> std::io::Result<LoopbackPortGuard>,
    bind_ephemeral: impl Fn() -> std::io::Result<TcpListener>,
) -> std::io::Result<ResilientGuardAcquisition> {
    let candidate_count = if is_windows {
        PORT_FALLBACK_CANDIDATE_SPAN as usize
    } else {
        1
    };
    let mut attempts = 0usize;
    let mut last_error: Option<std::io::Error> = None;

    for offset in 0..candidate_count {
        // checked_add：requested+offset 溢出（回绕到 0）时结束确定性区间，
        // port 0 永远不作为确定性候选，剩余回退交给 ephemeral 路径。
        let Some(port) = requested_port.checked_add(offset as u16) else {
            break;
        };
        attempts += 1;
        match acquire(port, state_dir) {
            Ok(guard) => {
                return Ok(ResilientGuardAcquisition {
                    guard,
                    requested_port,
                    effective_port: port,
                    attempts,
                });
            }
            // AddrInUse/WouldBlock 表示真实实例或锁冲突，保留单实例语义立即上抛。
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse
                    || error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err(error);
            }
            Err(error) => {
                // 非 Windows 不做候选回退：单次 acquire 失败原样上抛，不包装耗尽错误。
                if !is_windows {
                    return Err(error);
                }
                last_error = Some(error);
            }
        }
    }

    if is_windows {
        // 先拿到 ephemeral 端口并释放，再对同一端口走完整 acquire，保持锁文件语义。
        let ephemeral = bind_ephemeral()?;
        let port = ephemeral.local_addr()?.port();
        drop(ephemeral);
        attempts += 1;
        match acquire(port, state_dir) {
            Ok(guard) => {
                return Ok(ResilientGuardAcquisition {
                    guard,
                    requested_port,
                    effective_port: port,
                    attempts,
                });
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse
                    || error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err(error);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(exhausted_port_fallback_error(
        requested_port,
        attempts,
        last_error,
    ))
}

fn exhausted_port_fallback_error(
    requested_port: u16,
    attempts: usize,
    last_error: Option<std::io::Error>,
) -> std::io::Error {
    let (kind, source) = match last_error {
        Some(error) => (error.kind(), error.to_string()),
        None => (
            std::io::ErrorKind::PermissionDenied,
            "no candidate was attempted".to_string(),
        ),
    };
    std::io::Error::new(
        kind,
        format!(
            "failed to bind any loopback candidate near requested port {requested_port} \
             after {attempts} attempt(s): {source}; os error 10013 usually means the port \
             sits in the Windows excluded port range (often caused by Docker/WSL/WinNAT)"
        ),
    )
}

/// helper bind 结果：listener 之外携带 requested/effective 端口与尝试次数，
/// effective port 需沿 LaunchStatus 写回给 Manager。
#[derive(Debug)]
pub struct HelperBindResult {
    pub listener: TcpListener,
    pub requested_port: u16,
    pub effective_port: u16,
    pub attempts: usize,
}

/// 绑定 helper 端口；仅 Windows 且 bind_host 为 `127.0.0.1` 时启用候选回退，
/// 其他情况保持原有单次 bind 行为。
pub fn bind_helper_loopback_with_fallback(
    requested_port: u16,
    bind_host: &str,
) -> std::io::Result<HelperBindResult> {
    bind_helper_loopback_with_fallback_with(
        requested_port,
        cfg!(windows),
        bind_host,
        |host, port| TcpListener::bind((host, port)),
    )
}

fn bind_helper_loopback_with_fallback_with(
    requested_port: u16,
    is_windows: bool,
    bind_host: &str,
    bind: impl Fn(&str, u16) -> std::io::Result<TcpListener>,
) -> std::io::Result<HelperBindResult> {
    let use_fallback = is_windows && bind_host == "127.0.0.1";
    let candidate_count = if use_fallback {
        PORT_FALLBACK_CANDIDATE_SPAN as usize
    } else {
        1
    };
    let mut attempts = 0usize;
    let mut last_error: Option<std::io::Error> = None;

    for offset in 0..candidate_count {
        // checked_add：requested+offset 溢出（回绕到 0）时结束确定性区间，
        // port 0 永远不作为确定性候选，剩余回退交给 ephemeral 路径。
        let Some(port) = requested_port.checked_add(offset as u16) else {
            break;
        };
        attempts += 1;
        match bind(bind_host, port) {
            Ok(listener) => {
                return Ok(HelperBindResult {
                    listener,
                    requested_port,
                    effective_port: port,
                    attempts,
                });
            }
            Err(error) => {
                // 非回退场景保持原行为：任何错误直接上抛。
                if !use_fallback {
                    return Err(error);
                }
                // helper 无单实例语义：AddrInUse、excluded 及其他错误都继续候选，耗尽再上抛。
                last_error = Some(error);
            }
        }
    }

    if use_fallback {
        // 确定性候选耗尽后，尝试一个经验证可重绑的 ephemeral 端口。
        if let Some(port) = find_rebindable_loopback_port() {
            attempts += 1;
            match bind(bind_host, port) {
                Ok(listener) => {
                    return Ok(HelperBindResult {
                        listener,
                        requested_port,
                        effective_port: port,
                        attempts,
                    });
                }
                Err(error) => last_error = Some(error),
            }
        }
    }

    Err(exhausted_port_fallback_error(
        requested_port,
        attempts,
        last_error,
    ))
}

/// 通过 bind 0 获取 ephemeral 端口并确认释放后可再次绑定，最多尝试
/// `REBINDABLE_PORT_MAX_ATTEMPTS` 次；避免直接使用 Windows 可能从 excluded
/// port range 分配的端口。
pub fn find_rebindable_loopback_port() -> Option<u16> {
    for _ in 0..REBINDABLE_PORT_MAX_ATTEMPTS {
        let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        drop(listener);
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static GUARD_PORT_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resilient_guard_holds_lock_and_listener_when_requested_port_is_available() {
        let temp = tempfile::tempdir().unwrap();
        let port = find_available_loopback_port();

        let guard = acquire_resilient_loopback_port_guard_at(port, temp.path()).unwrap();

        assert!(guard.lock_path.is_some());
        assert!(guard._listener.is_some());
        assert!(guard.fallback_path().is_none());
    }

    #[test]
    fn resilient_guard_reports_lock_conflict_when_instance_lock_is_held() {
        let temp = tempfile::tempdir().unwrap();
        let port = find_available_loopback_port();
        let _guard = acquire_resilient_loopback_port_guard_at(port, temp.path()).unwrap();

        let second = acquire_resilient_loopback_port_guard_at(port, temp.path()).unwrap_err();

        assert_eq!(second.kind(), std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn resilient_guard_reports_conflict_when_requested_port_is_connectable() {
        let temp = tempfile::tempdir().unwrap();
        let error = acquire_resilient_loopback_port_guard_with(
            57319,
            temp.path(),
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "port busy",
                ))
            },
            |_| true,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn resilient_guard_uses_lock_fallback_when_requested_port_is_not_connectable() {
        let temp = tempfile::tempdir().unwrap();
        let guard = acquire_resilient_loopback_port_guard_with(
            57319,
            temp.path(),
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "stale port",
                ))
            },
            |_| false,
        )
        .unwrap();

        assert!(guard._listener.is_none());
        assert!(guard.fallback_path().is_some());

        let second = acquire_resilient_loopback_port_guard_with(
            57319,
            temp.path(),
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "stale port",
                ))
            },
            |_| false,
        )
        .unwrap_err();
        assert_eq!(second.kind(), std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn excluded_guard_port_falls_back_to_next_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let calls = Mutex::new(0usize);
        let acquisition = acquire_resilient_guard_with_port_fallback_with(
            57745,
            true,
            temp.path(),
            |port, state_dir| {
                *calls.lock().unwrap() += 1;
                if port == 57745 {
                    Err(std::io::Error::from_raw_os_error(10013))
                } else {
                    acquire_resilient_loopback_port_guard_at(port, state_dir)
                }
            },
            || acquire_loopback_port_guard(0),
        )
        .unwrap();
        assert_eq!(acquisition.requested_port, 57745);
        assert_eq!(acquisition.effective_port, 57746);
        assert_eq!(acquisition.attempts, 2);
    }

    #[test]
    fn addr_in_use_guard_port_does_not_try_later_candidates() {
        let calls = Mutex::new(0usize);
        let error = acquire_resilient_guard_with_port_fallback_with(
            57745,
            true,
            Path::new("/unused"),
            |_, _| {
                *calls.lock().unwrap() += 1;
                Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "in use"))
            },
            || panic!("must not bind ephemeral"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn excluded_helper_port_falls_back_to_next_candidate() {
        let result =
            bind_helper_loopback_with_fallback_with(57321, true, "127.0.0.1", |_, port| {
                if port == 57321 {
                    Err(std::io::Error::from_raw_os_error(10013))
                } else {
                    TcpListener::bind(("127.0.0.1", 0))
                }
            })
            .unwrap();
        assert_eq!(result.requested_port, 57321);
        assert!(result.effective_port != 0);
        assert_eq!(result.attempts, 2);
    }

    #[test]
    fn would_block_guard_port_does_not_try_later_candidates() {
        let calls = Mutex::new(0usize);
        let error = acquire_resilient_guard_with_port_fallback_with(
            57745,
            true,
            Path::new("/unused"),
            |_, _| {
                *calls.lock().unwrap() += 1;
                Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "lock held",
                ))
            },
            || panic!("must not bind ephemeral"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn non_windows_guard_error_is_returned_unwrapped() {
        let calls = Mutex::new(0usize);
        let error = acquire_resilient_guard_with_port_fallback_with(
            57745,
            false,
            Path::new("/unused"),
            |_, _| {
                *calls.lock().unwrap() += 1;
                Err(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
            },
            || panic!("must not bind ephemeral"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), "boom");
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn permission_denied_guard_port_continues_to_next_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let calls = Mutex::new(0usize);
        let acquisition = acquire_resilient_guard_with_port_fallback_with(
            57745,
            true,
            temp.path(),
            |port, state_dir| {
                *calls.lock().unwrap() += 1;
                if port == 57745 {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "access denied",
                    ))
                } else {
                    acquire_resilient_loopback_port_guard_at(port, state_dir)
                }
            },
            || acquire_loopback_port_guard(0),
        )
        .unwrap();
        assert_eq!(acquisition.requested_port, 57745);
        assert_eq!(acquisition.effective_port, 57746);
        assert_eq!(acquisition.attempts, 2);
    }

    #[test]
    fn helper_bind_on_non_loopback_host_does_not_fall_back() {
        let calls = Mutex::new(0usize);
        let error = bind_helper_loopback_with_fallback_with(57321, true, "0.0.0.0", |_, port| {
            *calls.lock().unwrap() += 1;
            assert_eq!(port, 57321);
            Err(std::io::Error::from_raw_os_error(10013))
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(10013));
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn is_excluded_port_error_matches_wsa_eacces_and_permission_denied() {
        assert!(is_excluded_port_error(&std::io::Error::from_raw_os_error(
            10013
        )));
        assert!(is_excluded_port_error(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "access denied",
        )));
        assert!(!is_excluded_port_error(&std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "in use",
        )));
        assert!(!is_excluded_port_error(&std::io::Error::new(
            std::io::ErrorKind::Other,
            "boom",
        )));
    }

    #[test]
    fn find_rebindable_loopback_port_returns_verified_port() {
        // 真实 loopback：bind 0 → drop → 同端口可再 bind。
        let port = find_rebindable_loopback_port().expect("loopback should yield a port");
        assert!(port > 0);
        let listener = TcpListener::bind(("127.0.0.1", port))
            .expect("rebindable port must bind again after release");
        drop(listener);
    }

    #[test]
    fn find_available_loopback_port_returns_verified_port() {
        // find_available_loopback_port 复用 rebindable 验证逻辑，
        // 返回的端口必须释放后可再绑定（或极端情况下为 0，本机不应出现）。
        let port = find_available_loopback_port();
        assert!(port > 0);
        assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn helper_candidates_near_u16_max_never_wrap_to_port_zero() {
        let attempted = Mutex::new(Vec::new());
        let result = bind_helper_loopback_with_fallback_with(65530, true, "127.0.0.1", |_, port| {
            attempted.lock().unwrap().push(port);
            if (65530..=65535).contains(&port) {
                Err(std::io::Error::from_raw_os_error(10013))
            } else {
                // ephemeral 路径：find_rebindable_loopback_port 给出的已验证端口应可绑定
                TcpListener::bind(("127.0.0.1", port))
            }
        })
        .unwrap();
        let attempted = attempted.lock().unwrap();
        // 65530+6 溢出后停止确定性候选：仅 6 个确定性尝试，且从不出现 port 0
        assert_eq!(attempted.len(), 7);
        assert!(!attempted.contains(&0));
        assert!(result.effective_port != 0);
        assert_eq!(result.attempts, 7);
    }

    #[test]
    fn guard_candidates_near_u16_max_never_wrap_to_port_zero() {
        let temp = tempfile::tempdir().unwrap();
        let attempted = Mutex::new(Vec::new());
        let acquisition = acquire_resilient_guard_with_port_fallback_with(
            65530,
            true,
            temp.path(),
            |port, state_dir| {
                attempted.lock().unwrap().push(port);
                if (65530..=65535).contains(&port) {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "access denied",
                    ))
                } else {
                    acquire_resilient_loopback_port_guard_at(port, state_dir)
                }
            },
            || acquire_loopback_port_guard(0),
        )
        .unwrap();
        let attempted = attempted.lock().unwrap();
        assert_eq!(attempted.len(), 7);
        assert!(!attempted.contains(&0));
        assert!(acquisition.effective_port != 0);
        assert_eq!(acquisition.attempts, 7);
    }

    #[test]
    fn launcher_guard_port_returns_base_when_no_env_override() {
        let _guard = guard_port_env_lock();
        _clear_guard_port_env_vars();
        let port = launcher_guard_port();
        // On non-Windows: LAUNCHER_GUARD_PORT_BASE + 0
        // On Windows: LAUNCHER_GUARD_PORT_BASE + USERNAME hash mod 1000
        assert!(port >= LAUNCHER_GUARD_PORT_BASE);
        assert!(port < LAUNCHER_GUARD_PORT_BASE + 1000);
    }

    #[test]
    fn manager_guard_port_returns_base_when_no_env_override() {
        let _guard = guard_port_env_lock();
        _clear_guard_port_env_vars();
        let port = manager_guard_port();
        assert!(port >= MANAGER_GUARD_PORT_BASE);
        assert!(port < MANAGER_GUARD_PORT_BASE + 1000);
    }

    #[test]
    fn launcher_guard_port_honors_env_override() {
        let _guard = guard_port_env_lock();
        _clear_guard_port_env_vars();
        unsafe { std::env::set_var("CODEX_PLUS_GUARD_PORT", "9999") };
        let port = launcher_guard_port();
        unsafe { std::env::remove_var("CODEX_PLUS_GUARD_PORT") };
        assert_eq!(port, 9999);
    }

    #[test]
    fn launcher_guard_port_honors_specific_env_override() {
        let _guard = guard_port_env_lock();
        _clear_guard_port_env_vars();
        unsafe { std::env::set_var("CODEX_PLUS_LAUNCHER_GUARD_PORT", "8888") };
        let port = launcher_guard_port();
        unsafe { std::env::remove_var("CODEX_PLUS_LAUNCHER_GUARD_PORT") };
        assert_eq!(port, 8888);
    }

    #[test]
    fn manager_guard_port_honors_specific_env_override() {
        let _guard = guard_port_env_lock();
        _clear_guard_port_env_vars();
        unsafe { std::env::set_var("CODEX_PLUS_MANAGER_GUARD_PORT", "7777") };
        let port = manager_guard_port();
        unsafe { std::env::remove_var("CODEX_PLUS_MANAGER_GUARD_PORT") };
        assert_eq!(port, 7777);
    }

    #[test]
    fn launcher_guard_port_honors_offset_env() {
        let _guard = guard_port_env_lock();
        _clear_guard_port_env_vars();
        unsafe { std::env::set_var("CODEX_PLUS_GUARD_PORT_OFFSET", "50") };
        let port = launcher_guard_port();
        unsafe { std::env::remove_var("CODEX_PLUS_GUARD_PORT_OFFSET") };
        assert_eq!(port, LAUNCHER_GUARD_PORT_BASE + 50);
    }

    fn guard_port_env_lock() -> MutexGuard<'static, ()> {
        GUARD_PORT_ENV_LOCK
            .lock()
            .expect("guard port env lock should not be poisoned")
    }
}

/// Clear all guard-port env vars to prevent cross-test contamination
/// when cargo runs tests in parallel threads.
fn _clear_guard_port_env_vars() {
    unsafe {
        let _ = std::env::remove_var("CODEX_PLUS_GUARD_PORT");
        let _ = std::env::remove_var("CODEX_PLUS_LAUNCHER_GUARD_PORT");
        let _ = std::env::remove_var("CODEX_PLUS_MANAGER_GUARD_PORT");
        let _ = std::env::remove_var("CODEX_PLUS_GUARD_PORT_OFFSET");
    }
}
