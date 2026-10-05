#!/usr/bin/env python3
"""Measure one rendered Nocterm process without global keyboard dispatch."""
import argparse
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import time


def command(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def wait_for(predicate, timeout, process, label):
    deadline = time.monotonic() + timeout
    next_tracking = 0.0
    while time.monotonic() < deadline:
        if time.monotonic() >= next_tracking:
            track_owned(process)
            next_tracking = time.monotonic() + 1
        if process.poll() is not None:
            raise RuntimeError(f"app exited ({process.returncode}) while waiting for {label}")
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise RuntimeError(f"timeout waiting for {label}")


def stats(pid):
    # comm can contain spaces and parentheses, so split after the final ')'.
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12]), int(fields[21]) * os.sysconf("SC_PAGE_SIZE")


def process_tree(root):
    entries = {}
    for path in Path("/proc").iterdir():
        if not path.name.isdigit():
            continue
        try:
            fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
            entries[int(path.name)] = (int(fields[1]), int(fields[19]), fields[0])
        except (FileNotFoundError, ProcessLookupError):
            pass
    owned = {root}
    while True:
        children = {pid for pid, (parent, _, _) in entries.items() if parent in owned}
        if children <= owned:
            break
        owned |= children
    return {pid: entries[pid] for pid in owned if pid in entries}


def track_owned(process):
    tree = process_tree(process.pid)
    if process.pid not in tree:
        return
    start = tree[process.pid][1]
    original = getattr(process, "_perf_root_start", start)
    if start != original:
        return
    process._perf_root_start = original
    tracked = getattr(process, "_perf_owned", {})
    tracked.update(tree)
    process._perf_owned = tracked


def owned_fd(pid, start_time):
    fd = None
    try:
        fd = os.pidfd_open(pid)
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        if int(fields[19]) == start_time and fields[0] != "Z":
            return fd
    except (FileNotFoundError, ProcessLookupError):
        pass
    if fd is not None:
        os.close(fd)
    return None


def cleanup(process):
    # PTYs and LocalExec intentionally have separate process groups. Track the
    # tree during the run, stop it before the final enumeration, and signal by
    # pidfd (immune to PID reuse). Tracking also retains reparented descendants
    # if the application exits unexpectedly before cleanup.
    if process.poll() is None:
        track_owned(process)
    tracked = getattr(process, "_perf_owned", {})
    descriptors = {}
    try:
        root = tracked.get(process.pid)
        if root:
            fd = owned_fd(process.pid, root[1])
            if fd is not None:
                descriptors[process.pid] = fd
                signal.pidfd_send_signal(fd, signal.SIGSTOP)
        for _ in range(5):
            if process.pid in descriptors:
                track_owned(process)
                tracked = process._perf_owned
            found = False
            for pid, (_, start_time, state) in tracked.items():
                if pid in descriptors or state == "Z":
                    continue
                fd = owned_fd(pid, start_time)
                if fd is not None:
                    try:
                        signal.pidfd_send_signal(fd, signal.SIGSTOP)
                        descriptors[pid] = fd
                        found = True
                    except ProcessLookupError:
                        os.close(fd)
            if not found:
                break
        for fd in descriptors.values():
            try:
                signal.pidfd_send_signal(fd, signal.SIGTERM)
                signal.pidfd_send_signal(fd, signal.SIGCONT)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass
        for fd in descriptors.values():
            try:
                signal.pidfd_send_signal(fd, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.wait(timeout=3)
    finally:
        for fd in descriptors.values():
            os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--home", type=Path, required=True)
    parser.add_argument("--legacy-hooks", action="store_true", help="allow old binaries without readiness acknowledgements")
    parser.add_argument("--reuse-home", action="store_true", help="skip per-run home copy")
    parser.add_argument("--mode", choices=["idle", "shell", "type", "flood"], default="shell")
    parser.add_argument("--agent", action="store_true")
    parser.add_argument("--chat", action="store_true")
    parser.add_argument("--ssh", default="", help="user@host SSH terminal scenario")
    parser.add_argument("--ssh-key", type=Path)
    parser.add_argument("--ssh-tabs", type=int, default=1)
    parser.add_argument("--keys", default="")
    parser.add_argument("--seconds", type=float, default=15)
    parser.add_argument("--warmup", type=float, default=6)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--workspace", default="4")
    parser.add_argument("--monitor", default="PERF-1")
    parser.add_argument("--run-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.seconds <= 0 or args.warmup < 0 or args.timeout <= 0:
        parser.error("seconds/timeout must be positive and warmup nonnegative")
    if args.ssh and (not args.ssh_key or args.ssh_tabs < 1):
        parser.error("--ssh requires --ssh-key and positive --ssh-tabs")
    args.run_dir.mkdir(parents=True, exist_ok=False)
    run = args.run_dir.resolve()
    home = args.home.resolve()
    if not args.reuse_home:
        shutil.copytree(home, run / "home")
        home = run / "home"
        settings = home / "config/settings.toml"
        import re
        source = settings.read_text()
        program = json.dumps(str(Path(__file__).resolve().parent / "shell.sh"))
        if re.search(r"(?m)^\[local\]$", source):
            section = re.search(r"(?ms)^\[local\]\n(.*?)(?=^\[|\Z)", source)
            body = section.group(1)
            if re.search(r"(?m)^program\s*=", body):
                body = re.sub(r"(?m)^program\s*=.*$", "program = " + program, body)
            else:
                body = "program = " + program + "\n" + body
            source = source[:section.start(1)] + body + source[section.end(1):]
        else:
            source += "\n[local]\nprogram = " + program + "\n"
        settings.write_text(source)
    env = os.environ.copy()
    for key in ["NOCTERM_PERF_AGENT", "NOCTERM_PERF_CHAT", "NOCTERM_PERF_KEYS", "NOCTERM_PERF_SSH", "NOCTERM_PERF_SSH_KEY", "NOCTERM_PERF_SSH_TABS"]:
        env.pop(key, None)
    env.update(NOCTERM_HOME=str(home), NOCTERM_PERF_LOCAL="1",
               NOCTERM_PERF_RUN=str(run), NOCTERM_PERF_MODE=args.mode)
    if args.agent:
        env["NOCTERM_PERF_AGENT"] = "1"
    if args.chat:
        env["NOCTERM_PERF_CHAT"] = "1"
    if args.ssh:
        env.update(NOCTERM_PERF_SSH=args.ssh,
                   NOCTERM_PERF_SSH_KEY=str(args.ssh_key.resolve()),
                   NOCTERM_PERF_SSH_TABS=str(args.ssh_tabs))
    if args.keys:
        env["NOCTERM_PERF_KEYS"] = args.keys
    # Compatibility with the preserved baseline shell; sequential runs only.
    Path("/tmp/perf-mode").write_text(args.mode + "\n")
    for marker in ["/tmp/perf-flood-start", "/tmp/perf-flood-done"]:
        Path(marker).unlink(missing_ok=True)
    previous_focus = json.loads(command("hyprctl", "activewindow", "-j"))
    process = None
    try:
        with (run / "app.log").open("w") as log:
            process = subprocess.Popen([str(args.binary.resolve())], env=env,
                                       stdout=log, stderr=log, start_new_session=True)
        track_owned(process)
        client = wait_for(lambda: next((c for c in json.loads(command("hyprctl", "clients", "-j"))
                                       if c["pid"] == process.pid), None),
                          args.timeout, process, "mapped window")
        command("hyprctl", "dispatch", f'hl.dsp.window.move({{ workspace = "{args.workspace}", follow = false, window = "pid:{process.pid}" }})')
        active = json.loads(command("hyprctl", "activewindow", "-j"))
        if active.get("pid") == process.pid and previous_focus.get("address"):
            command("hyprctl", "dispatch", "focuswindow", "address:" + previous_focus["address"])
        command("hyprctl", "dispatch", f'hl.dsp.window.fullscreen({{ window = "pid:{process.pid}" }})')

        def visible():
            clients = json.loads(command("hyprctl", "clients", "-j"))
            monitors = json.loads(command("hyprctl", "monitors", "-j"))
            mapped = next((c for c in clients if c["pid"] == process.pid), None)
            monitor = next((m for m in monitors if m["name"] == args.monitor), None)
            return (mapped and monitor and mapped.get("mapped", True) and
                    not mapped.get("hidden", False) and monitor.get("dpmsStatus", True) and
                    not monitor.get("disabled", False) and mapped["monitor"] == monitor["id"] and
                    mapped["workspace"]["id"] == monitor["activeWorkspace"]["id"])

        wait_for(visible, args.timeout, process, "window on active benchmark monitor")
        client = next(c for c in json.loads(command("hyprctl", "clients", "-j")) if c["pid"] == process.pid)
        if not args.legacy_hooks:
            wait_for(lambda: (run / "app-ready").exists(), args.timeout, process, "app readiness")
            wait_for(lambda: (run / "shell-ready").exists(), args.timeout, process, "shell readiness")
            if args.ssh:
                wait_for(lambda: (run / "ssh-ready").exists(), args.timeout, process, "SSH session readiness")
            if args.keys:
                wait_for(lambda: (run / "keys-count").exists(), args.timeout, process, "keys in a focused connected terminal")
        if args.mode == "flood":
            # A producer marker cannot establish that the terminal drained output.
            # New measurement-only hooks acknowledge a unique terminal title only
            # after emulator.advance has consumed the final sentinel.
            wait_for(lambda: (run / "flood-start").exists(), args.timeout, process, "producer start")
        else:
            deadline = time.monotonic() + args.warmup
            wait_for(lambda: time.monotonic() >= deadline, args.warmup + 1, process, "warmup")
        key_start = [int(n) for n in (run / "keys-count").read_text().split()] if args.keys and not args.legacy_hooks else None
        cpu0, _ = stats(process.pid)
        start = time.monotonic()
        if args.mode == "flood":
            wait_for(lambda: (run / "flood-drained").exists(), args.timeout, process, "emulator drain acknowledgement")
        else:
            deadline = start + args.seconds
            wait_for(lambda: time.monotonic() >= deadline, args.seconds + 1, process, "measurement interval")
        end = time.monotonic()
        cpu1, rss = stats(process.pid)
        if not visible():
            raise RuntimeError("window became hidden during measurement")
        key_end = [int(n) for n in (run / "keys-count").read_text().split()] if key_start else None
        if key_end and (key_end[0] <= key_start[0] or key_end[0] != key_end[1] or key_end[0] != key_end[2]):
            raise RuntimeError("keys did not reach a focused connected terminal")
        elapsed = end - start
        cpu_seconds = (cpu1 - cpu0) / os.sysconf("SC_CLK_TCK")
        result = {"binary": str(args.binary.resolve()), "mode": args.mode,
                  "agent": args.agent, "chat": args.chat, "keys": args.keys,
                  "wall_seconds": elapsed, "cpu_seconds": cpu_seconds,
                  "cpu_percent_one_core": 100 * cpu_seconds / elapsed, "rss_bytes": rss,
                  "viewport": client["size"], "pid": process.pid,
                  "readiness_acknowledged": not args.legacy_hooks,
                  "keys_start": key_start, "keys_end": key_end,
                  "ssh": bool(args.ssh), "ssh_tabs": args.ssh_tabs if args.ssh else 0}
        (run / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        command("grim", "-o", args.monitor, str(run / "screenshot.png"))
        print(json.dumps(result))
    finally:
        if process is not None:
            cleanup(process)


if __name__ == "__main__":
    main()
