#!/usr/bin/env python3
"""Record a real GridBash/Codex session in an isolated fullscreen terminal.

The script creates a disposable git repository in the user's temp directory,
prepares six genuine git worktrees, launches six Codex sessions through
GridBash, selects a four-pane subset, and sends each selected session one
bounded proof prompt. Temporary per-pane auth homes are deleted after capture.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import pyautogui
import psutil
import win32con
import win32gui
import win32process
from pywinauto import Application


PROJECT_DIR = Path(__file__).resolve().parents[1]
DEMO_REPO = Path(tempfile.gettempdir()) / "gridbash-launch-video-demo"
DEMO_CODEX_HOME = DEMO_REPO / ".codex-demo-home"
DEMO_CODEX_WRAPPER = DEMO_REPO / "codex-demo.cmd"
RAW_VIDEO = PROJECT_DIR / "raw" / "gridbash-product-demo-raw.mp4"
FINAL_VIDEO = PROJECT_DIR / "assets" / "product-demo.mp4"
POSTER = PROJECT_DIR / "assets" / "product-demo-poster.png"
DRY_RUN_STILL = PROJECT_DIR / "snapshots" / "gridbash-live-demo-dry-run.png"
DEMO_CONFIG = DEMO_REPO / "gridbash-demo.toml"

WINDOW_CLASS = "GridBashDemoCapture"
WINDOW_TITLE = "GridBash Product Demo Capture"
WEZTERM = Path(r"C:\Program Files\WezTerm\wezterm.exe")


def checked_run(args: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, check=True, text=True, **kwargs)


def command_path(name: str, fallback: Path | None = None) -> str:
    resolved = shutil.which(name)
    if resolved:
        return resolved
    if fallback and fallback.is_file():
        return str(fallback)
    raise RuntimeError(f"Required command not found: {name}")


def prepare_demo_repo() -> None:
    temp_root = Path(tempfile.gettempdir()).resolve()
    target = DEMO_REPO.resolve()
    if temp_root not in target.parents:
        raise RuntimeError(f"Refusing to prepare demo outside temp: {target}")

    git = command_path("git")
    if not (DEMO_REPO / ".git").exists():
        DEMO_REPO.mkdir(parents=True, exist_ok=True)
        checked_run([git, "-C", str(DEMO_REPO), "init", "-b", "main"])
        checked_run(
            [
                git,
                "-C",
                str(DEMO_REPO),
                "-c",
                "user.name=GridBash Demo",
                "-c",
                "user.email=demo@gridbash.local",
                "commit",
                "--allow-empty",
                "-m",
                "demo: initialize workspace",
            ]
        )

    inside = checked_run(
        [git, "-C", str(DEMO_REPO), "rev-parse", "--is-inside-work-tree"],
        capture_output=True,
    ).stdout.strip()
    if inside != "true":
        raise RuntimeError(f"Demo path is not a git repository: {DEMO_REPO}")

    DEMO_CONFIG.write_text(
        "[profiles.codex-demo]\n"
        f"command = \"{DEMO_CODEX_WRAPPER.as_posix()}\"\n"
        "args = [\"--disable\", \"apps\", \"--sandbox\", \"read-only\", "
        "\"--ask-for-approval\", \"never\", "
        "\"-c\", \"model_reasoning_effort=\\\"none\\\"\"]\n"
        "title = \"Codex\"\n",
        encoding="utf-8",
    )

    worktree_root = DEMO_REPO / ".worktrees"
    worktree_root.mkdir(exist_ok=True)
    for number in range(1, 7):
        branch = f"launch/main-pane-{number:02d}"
        folder = worktree_root / f"launch-main-{number:02d}"
        existing = checked_run(
            [git, "-C", str(DEMO_REPO), "branch", "--list", branch],
            capture_output=True,
        ).stdout.strip()
        if folder.is_dir() and existing:
            continue
        if folder.exists() or existing:
            raise RuntimeError(
                f"Incomplete disposable worktree state for {branch}; remove {DEMO_REPO} and retry"
            )
        checked_run(
            [
                git,
                "-C",
                str(DEMO_REPO),
                "worktree",
                "add",
                "-b",
                branch,
                str(folder),
                "HEAD",
            ]
        )


def prepare_demo_codex_home() -> None:
    global_codex_home = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex"))
    auth_source = global_codex_home / "auth.json"
    if not auth_source.is_file():
        raise RuntimeError(f"Codex auth file not found: {auth_source}")
    codex_cmd = Path(command_path("codex"))

    cleanup_demo_codex_home()
    try:
        DEMO_CODEX_HOME.mkdir(parents=True)
        trusted_path = str(DEMO_REPO).lower().replace("'", "''")
        for number in range(1, 7):
            pane_home = DEMO_CODEX_HOME / f"launch-main-{number:02d}"
            pane_home.mkdir()
            shutil.copy2(auth_source, pane_home / "auth.json")
            for name in ("models_cache.json", "version.json"):
                source = global_codex_home / name
                if source.is_file():
                    shutil.copy2(source, pane_home / name)
            (pane_home / "config.toml").write_text(
                "model = \"gpt-5.6-sol\"\n"
                "model_reasoning_effort = \"none\"\n\n"
                f"[projects.'{trusted_path}']\n"
                "trust_level = \"trusted\"\n",
                encoding="utf-8",
            )
        DEMO_CODEX_WRAPPER.write_text(
            "@echo off\r\n"
            "for %%I in (\"%CD%\") do set \"GRIDBASH_DEMO_PANE=%%~nxI\"\r\n"
            f"set \"CODEX_HOME={DEMO_CODEX_HOME}\\%GRIDBASH_DEMO_PANE%\"\r\n"
            f"call \"{codex_cmd}\" %*\r\n",
            encoding="utf-8",
        )
    except Exception:
        cleanup_demo_codex_home()
        raise


def cleanup_demo_codex_home() -> None:
    target = DEMO_CODEX_HOME.resolve()
    root = DEMO_REPO.resolve()
    if root not in target.parents:
        raise RuntimeError(f"Refusing to clean Codex home outside demo repo: {target}")
    if DEMO_CODEX_HOME.exists():
        shutil.rmtree(DEMO_CODEX_HOME)


def find_window_handle(process_id: int, timeout: float = 8.0) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        matches: list[int] = []
        try:
            root = psutil.Process(process_id)
            process_ids = {process_id, *(child.pid for child in root.children(recursive=True))}
        except psutil.Error:
            process_ids = {process_id}

        def visit(hwnd: int, _: object) -> None:
            _, window_process_id = win32process.GetWindowThreadProcessId(hwnd)
            if not win32gui.IsWindowVisible(hwnd) or window_process_id not in process_ids:
                return
            try:
                process_name = psutil.Process(window_process_id).name().lower()
            except psutil.Error:
                return
            if process_name == "wezterm-gui.exe" and win32gui.GetWindow(hwnd, win32con.GW_OWNER) == 0:
                matches.append(hwnd)

        win32gui.EnumWindows(visit, None)
        if matches:
            return matches[0]
        time.sleep(0.2)
    raise RuntimeError(f"Could not find the visible window for process tree {process_id}")


def focus_and_fullscreen(hwnd: int) -> None:
    win32gui.ShowWindow(hwnd, win32con.SW_RESTORE)
    win32gui.SetWindowPos(
        hwnd,
        win32con.HWND_TOPMOST,
        0,
        0,
        1920,
        1080,
        win32con.SWP_SHOWWINDOW,
    )
    for _ in range(10):
        try:
            Application(backend="win32").connect(handle=hwnd).window(handle=hwnd).set_focus()
            win32gui.BringWindowToTop(hwnd)
            win32gui.SetForegroundWindow(hwnd)
        except Exception:
            pass
        if win32gui.GetForegroundWindow() == hwnd:
            break
        time.sleep(0.2)
    time.sleep(0.6)
    left, top, right, bottom = win32gui.GetWindowRect(hwnd)
    width, height = right - left, bottom - top
    if width < 1900 or height < 1030:
        raise RuntimeError(
            f"Dedicated terminal did not cover the display: {(left, top, right, bottom)}"
        )
    if win32gui.GetForegroundWindow() != hwnd:
        foreground = win32gui.GetForegroundWindow()
        raise RuntimeError(
            "Dedicated terminal is not foreground; refusing input/capture "
            f"(target={hwnd}, foreground={foreground})"
        )
    print(
        "DEMO_WINDOW "
        f"hwnd={hwnd} title={win32gui.GetWindowText(hwnd)!r} "
        f"rect={(left, top, right, bottom)}",
        flush=True,
    )


def require_demo_foreground(hwnd: int) -> None:
    if win32gui.GetForegroundWindow() != hwnd:
        raise RuntimeError("Demo window lost foreground; refusing to send input or capture")
    left, top, right, bottom = win32gui.GetWindowRect(hwnd)
    if right - left < 1900 or bottom - top < 1030:
        raise RuntimeError("Demo window no longer covers the display")


def launch_terminal() -> tuple[subprocess.Popen[bytes], int, str]:
    if not WEZTERM.is_file():
        raise RuntimeError(f"WezTerm not found at {WEZTERM}")
    prepare_demo_codex_home()

    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    workspace = f"gridbash-product-demo-{os.getpid()}-{time.time_ns()}"
    try:
        process = subprocess.Popen(
            [
                str(WEZTERM),
                "--config",
                "enable_tab_bar=false",
                "--config",
                "font_size=14.0",
                "--config",
                'window_decorations="NONE"',
                "start",
                "--always-new-process",
                "--class",
                WINDOW_CLASS,
                "--workspace",
                workspace,
                "--position",
                "0,0",
                "--cwd",
                str(DEMO_REPO),
                "--",
                "powershell.exe",
                "-NoLogo",
                "-NoExit",
            ],
            creationflags=creationflags,
        )
    except Exception:
        cleanup_demo_codex_home()
        raise
    try:
        hwnd = find_window_handle(process.pid)
        win32gui.SetWindowText(hwnd, WINDOW_TITLE)
        focus_and_fullscreen(hwnd)

        command = (
            "gridbash 2x3 --config gridbash-demo.toml "
            "--profile codex-demo --worktrees --worktree-prefix launch"
        )
        return process, hwnd, command
    except Exception:
        terminate_process(process)
        cleanup_demo_codex_home()
        raise


def hotkey(hwnd: int, *keys: str) -> None:
    require_demo_foreground(hwnd)
    pyautogui.hotkey(*keys)


def select_pane(hwnd: int, x: int, y: int) -> None:
    require_demo_foreground(hwnd)
    pyautogui.click(x=x, y=y, button="right")


def wait_until(start: float, timestamp: float) -> None:
    remaining = start + timestamp - time.monotonic()
    if remaining > 0:
        time.sleep(remaining)


def type_launch_command(hwnd: int, command: str) -> None:
    require_demo_foreground(hwnd)
    pyautogui.write(command, interval=0.004)


def drive_recording(start: float, hwnd: int) -> None:
    # Launch reveal.
    wait_until(start, 0.55)
    require_demo_foreground(hwnd)
    pyautogui.press("enter")

    # Scene 4 is sourced from 7.00s. Build a real four-pane selection in time
    # with the launch video's focus/select/broadcast callouts.
    wait_until(start, 8.95)
    hotkey(hwnd, "alt", "right")
    wait_until(start, 9.27)
    select_pane(hwnd, 950, 260)
    wait_until(start, 9.84)
    hotkey(hwnd, "alt", "right")
    wait_until(start, 10.16)
    select_pane(hwnd, 1580, 260)
    wait_until(start, 10.74)
    hotkey(hwnd, "alt", "down")
    wait_until(start, 11.06)
    select_pane(hwnd, 1580, 760)
    wait_until(start, 11.60)
    hotkey(hwnd, "alt", "left")
    wait_until(start, 11.92)
    select_pane(hwnd, 950, 760)

    # Submit one real prompt only to the selected Codex sessions.
    wait_until(start, 12.12)
    require_demo_foreground(hwnd)
    pyautogui.write("Reply with exactly ROUTED.", interval=0.014)
    wait_until(start, 12.68)
    require_demo_foreground(hwnd)
    pyautogui.press("enter")
    wait_until(start, 24.0)


def start_ffmpeg_capture(duration: float) -> subprocess.Popen[bytes]:
    ffmpeg = command_path("ffmpeg")
    RAW_VIDEO.parent.mkdir(parents=True, exist_ok=True)
    return subprocess.Popen(
        [
            ffmpeg,
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-f",
            "gdigrab",
            "-framerate",
            "30",
            "-draw_mouse",
            "0",
            "-video_size",
            "1920x1080",
            "-offset_x",
            "0",
            "-offset_y",
            "0",
            "-i",
            "desktop",
            "-t",
            f"{duration:.2f}",
            "-an",
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p4",
            "-rc:v",
            "vbr",
            "-cq:v",
            "18",
            "-b:v",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            str(RAW_VIDEO),
        ],
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )


def conform_capture(pre_roll: float) -> None:
    ffmpeg = command_path("ffmpeg")
    FINAL_VIDEO.parent.mkdir(parents=True, exist_ok=True)
    checked_run(
        [
            ffmpeg,
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-ss",
            f"{pre_roll:.3f}",
            "-i",
            str(RAW_VIDEO),
            "-t",
            "18.000",
            "-vf",
            "fps=30,scale=1920:1080:flags=lanczos,setsar=1,format=yuv420p",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-g",
            "30",
            "-keyint_min",
            "30",
            "-sc_threshold",
            "0",
            "-movflags",
            "+faststart",
            str(FINAL_VIDEO),
        ]
    )
    checked_run(
        [
            ffmpeg,
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-i",
            str(FINAL_VIDEO),
            "-ss",
            "17.90",
            "-frames:v",
            "1",
            "-vf",
            "scale=1920:1080:flags=lanczos",
            str(POSTER),
        ]
    )


def terminate_process(process: subprocess.Popen[bytes] | None) -> None:
    if not process or process.poll() is not None:
        return
    try:
        root = psutil.Process(process.pid)
        descendants = root.children(recursive=True)
        for child in reversed(descendants):
            child.terminate()
        root.terminate()
        _, alive = psutil.wait_procs([*descendants, root], timeout=5)
        for item in alive:
            item.kill()
        psutil.wait_procs(alive, timeout=5)
    except psutil.Error:
        process.terminate()


def dry_run() -> None:
    terminal: subprocess.Popen[bytes] | None = None
    try:
        terminal, hwnd, launch_command = launch_terminal()
        type_launch_command(hwnd, launch_command)
        pyautogui.press("enter")
        time.sleep(8.0)
        hotkey(hwnd, "alt", "right")
        select_pane(hwnd, 950, 260)
        hotkey(hwnd, "alt", "right")
        select_pane(hwnd, 1580, 260)
        hotkey(hwnd, "alt", "down")
        select_pane(hwnd, 1580, 760)
        hotkey(hwnd, "alt", "left")
        select_pane(hwnd, 950, 760)
        time.sleep(1.0)
        require_demo_foreground(hwnd)
        DRY_RUN_STILL.parent.mkdir(parents=True, exist_ok=True)
        pyautogui.screenshot(str(DRY_RUN_STILL))
        print(DRY_RUN_STILL)
    finally:
        terminate_process(terminal)
        cleanup_demo_codex_home()


def record() -> None:
    terminal: subprocess.Popen[bytes] | None = None
    capture: subprocess.Popen[bytes] | None = None
    pre_roll = 1.0
    try:
        terminal, hwnd, launch_command = launch_terminal()
        type_launch_command(hwnd, launch_command)
        require_demo_foreground(hwnd)
        capture = start_ffmpeg_capture(pre_roll + 24.25)
        time.sleep(pre_roll)
        start = time.monotonic()
        drive_recording(start, hwnd)
        return_code = capture.wait(timeout=8)
        if return_code != 0:
            raise RuntimeError(f"FFmpeg capture failed with exit code {return_code}")
    finally:
        terminate_process(capture)
        terminate_process(terminal)
        cleanup_demo_codex_home()
    conform_capture(pre_roll)
    print(FINAL_VIDEO)
    print(POSTER)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("prepare", "dry-run", "record"))
    args = parser.parse_args()

    pyautogui.FAILSAFE = True
    pyautogui.PAUSE = 0.035
    prepare_demo_repo()
    if args.mode == "dry-run":
        dry_run()
    elif args.mode == "record":
        record()
    else:
        print(DEMO_REPO)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("Recording cancelled.", file=sys.stderr)
        raise SystemExit(130)
