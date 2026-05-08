from __future__ import annotations

import argparse
import traceback
from pathlib import Path

from codex_session_delete.helper_server import HelperServer
from codex_session_delete.installers import InstallOptions, install_codex_plus_plus, uninstall_codex_plus_plus
from codex_session_delete.launcher import launch_and_inject


def add_launch_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--app-dir", type=Path, default=None)
    parser.add_argument("--db", type=Path, default=Path.home() / ".codex" / "state_5.sqlite", help="SQLite database path for local deletion fallback")
    parser.add_argument("--backup-dir", type=Path, default=Path.home() / ".codex-session-delete" / "backups")
    parser.add_argument("--debug-port", type=int, default=9229)
    parser.add_argument("--helper-port", type=int, default=57321)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Launch and install Codex++ for Codex App")
    subparsers = parser.add_subparsers(dest="command")

    launch_parser = subparsers.add_parser("launch", help="Launch Codex with Codex++ injection")
    add_launch_arguments(launch_parser)

    install_parser = subparsers.add_parser("install", help="Install the Codex++ launcher entry point")
    install_parser.add_argument("--install-root", type=Path, default=None)
    install_parser.add_argument("--launcher-command", default=None)

    setup_parser = subparsers.add_parser("setup", help="Install Codex++ with defaults")
    setup_parser.add_argument("--install-root", type=Path, default=None)

    uninstall_parser = subparsers.add_parser("uninstall", help="Remove the Codex++ launcher entry point")
    uninstall_parser.add_argument("--install-root", type=Path, default=None)
    uninstall_parser.add_argument("--remove-data", action="store_true")

    remove_parser = subparsers.add_parser("remove", help="Remove Codex++ with defaults")
    remove_parser.add_argument("--install-root", type=Path, default=None)
    remove_parser.add_argument("--remove-data", action="store_true")

    add_launch_arguments(parser)
    return parser




def launch_log_path() -> Path:
    return Path.home() / ".codex-session-delete" / "launcher.log"


def log_launch_failure(exc: BaseException) -> None:
    path = launch_log_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(traceback.format_exception(type(exc), exc, exc.__traceback__)), encoding="utf-8")


def wait_for_shutdown(server: HelperServer, codex_proc) -> None:
    try:
        if codex_proc is None:
            import subprocess as _sp
            import time as _time
            while True:
                result = _sp.run(["pgrep", "-f", "^/Applications/Codex\\.app/Contents/MacOS/Codex"], capture_output=True)
                if result.returncode != 0:
                    break
                _time.sleep(2)
        else:
            codex_proc.wait()
    except KeyboardInterrupt:
        pass
    finally:
        server.shutdown()


def run_launch(args: argparse.Namespace) -> int:
    try:
        server, codex_proc = launch_and_inject(args.app_dir, args.db, args.backup_dir, args.debug_port, args.helper_port)
    except Exception as exc:
        log_launch_failure(exc)
        raise
    print(f"Codex session delete helper running on http://127.0.0.1:{server.port}")
    print("Keep this terminal open while using the delete buttons. Press Ctrl+C to stop.")
    wait_for_shutdown(server, codex_proc)
    return 0


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command in {"install", "setup"}:
        install_codex_plus_plus(InstallOptions(install_root=args.install_root, launcher_command=getattr(args, "launcher_command", None)))
        return 0
    if args.command in {"uninstall", "remove"}:
        uninstall_codex_plus_plus(InstallOptions(install_root=args.install_root, remove_data=args.remove_data))
        return 0
    return run_launch(args)


if __name__ == "__main__":
    raise SystemExit(main())
