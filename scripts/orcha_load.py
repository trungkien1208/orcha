#!/usr/bin/env python3
"""Exercise isolated Orcha projects with synthetic terminal producers, no paid agents.

Requires tmux and lsof on Unix. Reports API-to-terminal input latency (including
CLI overhead), sampled server/client CPU and RSS, and verifies every producer.
Never addresses or changes the caller's session, configuration, tmux server or
home folder: the isolated server gets a home of its own, so it cannot read the
caller's agent sign-ins (Keychain, ~/.claude, ~/.codex, ~/.cursor) or call paid
APIs with them.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import shlex
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import uuid


PRODUCER = r'''
import os, select, sys, time, tty
tty.setcbreak(sys.stdin.fileno())
gate, label, hz = sys.argv[1], sys.argv[2], float(sys.argv[3])
while not os.path.exists(gate): time.sleep(.01)
seq, received = 0, b""
while True:
    ready, _, _ = select.select([sys.stdin], [], [], 1 / hz)
    if ready: received = (received + os.read(sys.stdin.fileno(), 4096))[-32:]
    seq += 1
    title = "|/-\\"[seq % 4] + " " + label
    sys.stdout.write("\033]0;" + title + "\007\033[H\033[2J"
                     + "load-output-" + label + " " + str(seq) + "\r\n"
                     + "INPUT:" + received.decode(errors="replace") + "\r\n")
    sys.stdout.flush()
'''


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--projects", type=positive, default=15)
    parser.add_argument("--agents", type=positive, default=3, help="synthetic producers per project")
    parser.add_argument("--seconds", type=positive, default=10)
    parser.add_argument("--hz", type=positive, default=20)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("requires a Unix host")
    for tool in ["tmux", "lsof", "ps"]:
        if not shutil.which(tool):
            parser.error(f"required tool not found: {tool}")
    binary = args.binary.resolve(strict=True)
    name = "ol-" + uuid.uuid4().hex[:12]
    env = {key: value for key, value in os.environ.items()
           if not key.startswith("HERDR_") and key != "ORCHA_APP"}
    # Preserve nested-session protection; enable it only in the temporary config.
    if "HERDR_ENV" in os.environ:
        env["HERDR_ENV"] = os.environ["HERDR_ENV"]
    report: dict = {"session": name, "projects": args.projects,
                    "producers": args.projects * args.agents,
                    "hz_per_producer": args.hz, "sample_seconds": args.seconds,
                    "synthetic_agents": True, "geometry": [120, 42]}
    with tempfile.TemporaryDirectory(prefix="orcha-load-", dir="/var/tmp") as folder:
        root = Path(folder)
        config = root / "config.toml"
        config.write_text('onboarding = false\n[experimental]\nallow_nested = true\n'
                          '[theme]\nname = "orcha-dark"\n[orcha]\nupdate_plugins = false\n'
                          'accounts_command = "printf \'[]\'"\n')
        home = root / "home"
        # A home of its own: the real one holds agent sign-ins and Keychain
        # access the server would otherwise pick up and use.
        overrides = dict(ORCHA_APP="1", HERDR_CONFIG_PATH=str(config), HERDR_DISABLE_SOUND="1",
                         HOME=str(home), XDG_CONFIG_HOME=str(root / "config"),
                         XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"),
                         XDG_STATE_HOME=str(root / "state"),
                         XDG_RUNTIME_DIR=str(root / "run"), SHELL="/bin/sh",
                         PATH=os.environ["PATH"])
        if "HERDR_ENV" in env:
            overrides["HERDR_ENV"] = env["HERDR_ENV"]
        env.update(overrides)
        for folder_name in ("home", "run"):
            (root / folder_name).mkdir(mode=0o700)
        # A private tmux server: never the caller's sessions, never its config.
        tmux = ["tmux", "-L", name, "-f", os.devnull]
        control_env = dict(env, HERDR_SESSION=name)

        def command(*parts: str, check: bool = True) -> subprocess.CompletedProcess:
            return subprocess.run([str(binary), *parts], env=control_env, text=True,
                                  capture_output=True, timeout=15, check=check)

        def api(*parts: str) -> dict:
            answer = json.loads(command(*parts).stdout)
            if answer.get("error"):
                raise RuntimeError(answer["error"])
            return answer.get("result", answer)

        def read(pane: str) -> str:
            return command("pane", "read", pane, "--source", "visible", "--format", "text").stdout

        command("config", "check")
        report["binary"] = str(binary)
        report["version"] = command("--version").stdout.strip()
        producer = root / "producer.py"
        producer.write_text(PRODUCER)
        gate = root / "start"
        removed = {key for key in os.environ if key.startswith("HERDR_")}
        removed.update(["ORCHA_APP", "HERDR_SESSION", "HERDR_SOCKET_PATH",
                        "HERDR_CLIENT_SOCKET_PATH", "HERDR_BIN_PATH"])
        launch = shlex.join(["env", *[part for key in sorted(removed) for part in ("-u", key)],
                             *[f"{key}={value}" for key, value in overrides.items()],
                             str(binary), "--session", name])
        started = False
        try:
            subprocess.run([*tmux, "new-session", "-d", "-s", name, "-x", "120", "-y", "42",
                            launch], check=True, timeout=15)
            started = True
            deadline = time.monotonic() + 20
            while True:
                ready = command("pane", "list", check=False)
                if ready.returncode == 0:
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError("isolated session API did not become ready")
                time.sleep(.05)
            panes = []
            for project in range(args.projects):
                cwd = root / f"project-{project}"
                cwd.mkdir()
                workspace = api("workspace", "create", "--cwd", str(cwd), "--label", cwd.name,
                                "--focus" if project == 0 else "--no-focus")
                panes.append(workspace["root_pane"]["pane_id"])
                for agent in range(1, args.agents):
                    tab = api("tab", "create", "--workspace", workspace["workspace"]["workspace_id"],
                              "--cwd", str(cwd), "--label", f"producer-{agent}", "--no-focus")
                    panes.append(tab["root_pane"]["pane_id"])
            for index, pane in enumerate(panes):
                command("pane", "run", pane, sys.executable, str(producer), str(gate),
                        f"p{index}", str(args.hz))
            gate.touch()
            session = next(s for s in json.loads(command("session", "list", "--json").stdout)["sessions"]
                           if s["name"] == name)
            server = subprocess.check_output(["lsof", "-t", session["socket_path"]], text=True).split()[0]
            client = subprocess.check_output([*tmux, "list-panes", "-t", name, "-F", "#{pane_pid}"],
                                             text=True).strip()
            # Wait for every producer before measuring, not for a guessed startup delay.
            for index, pane in enumerate(panes):
                deadline = time.monotonic() + 10
                while f"load-output-p{index} " not in read(pane):
                    if time.monotonic() > deadline:
                        raise RuntimeError(f"producer {index} did not emit output")
                    time.sleep(.05)
            latency, cpu, rss = [], [], []
            deadline = time.monotonic() + args.seconds
            while time.monotonic() < deadline:
                token = uuid.uuid4().hex[:12]
                before = time.monotonic()
                command("pane", "send-text", panes[0], token)
                while token not in read(panes[0]).replace("\n", "").replace("\r", ""):
                    if time.monotonic() - before > 3:
                        raise RuntimeError("input echo exceeded 3 seconds under load")
                    time.sleep(.005)
                latency.append((time.monotonic() - before) * 1000)
                sample = subprocess.check_output(["ps", "-p", f"{server},{client}", "-o", "%cpu=,rss="],
                                                 text=True).splitlines()
                values = [line.split() for line in sample if line.strip()]
                if len(values) != 2:
                    raise RuntimeError("server or client exited during load")
                cpu.append(sum(float(v[0]) for v in values))
                rss.append(sum(int(v[1]) for v in values))
                time.sleep(min(.25, max(0, deadline - time.monotonic())))
            # ps %cpu is a host-dependent rolling average, not a precise interval counter.
            ordered = sorted(latency)
            report.update(input_latency_ms_median=statistics.median(latency),
                          input_latency_ms_p95=ordered[math.ceil(len(ordered) * .95) - 1],
                          input_samples=len(latency), cpu_percent_ps_mean=statistics.mean(cpu),
                          rss_kib_peak=max(rss), verified_producers=len(panes))
        except Exception as error:
            report["error"] = str(error)
            if 'panes' in locals() and panes:
                result = command("pane", "read", panes[0], "--source", "visible", "--format", "text", check=False)
                report["last_pane_read"] = result.stdout[-4000:]
            raise
        finally:
            # Never raise from here: that would hide the run's own error and
            # skip the report. A cleanup problem is recorded in the report.
            if started:
                report["cleaned_up"] = cleanup(command, tmux, name, report)
            write_report(args.report, report)
    print(json.dumps(report, indent=2))
    if report.get("cleaned_up") is False:
        sys.exit(f"temporary session {name} was not cleaned up: {report.get('cleanup_error')}")


def cleanup(command, tmux: list[str], name: str, report: dict) -> bool:
    """Stops and deletes the temporary session and its tmux server; True when nothing remains."""
    problems = []
    try:
        command("session", "stop", name, check=False)
        for _ in range(30):
            if command("session", "delete", name, check=False).returncode == 0:
                break
            time.sleep(.1)
        else:
            problems.append(f"could not delete temporary session {name}")
    except Exception as error:  # a stuck CLI must not stop the rest of the cleanup
        problems.append(f"deleting the session failed: {error}")
    try:
        subprocess.run([*tmux, "kill-server"], capture_output=True, timeout=10)
    except Exception as error:
        problems.append(f"stopping the private tmux server failed: {error}")
    try:
        remaining = json.loads(command("session", "list", "--json").stdout)["sessions"]
        if any(s["name"] == name for s in remaining):
            problems.append(f"temporary session {name} remains")
    except Exception as error:
        problems.append(f"listing sessions failed: {error}")
    if problems:
        report["cleanup_error"] = "; ".join(problems)
    return not problems


def write_report(path: Path | None, report: dict) -> None:
    """Writes the report when asked for one; a failure here is printed, never raised."""
    if not path:
        return
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report, indent=2) + "\n")
    except OSError as error:
        print(f"could not write the report to {path}: {error}", file=sys.stderr)


if __name__ == "__main__":
    main()
