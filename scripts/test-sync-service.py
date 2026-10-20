#!/usr/bin/env python3
"""Two real GNOME processes on private buses share a real local sync server."""
import json
import os
from pathlib import Path
import re
import select
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/debug"
NAME = "org.queuefocus.QueueFocus"
OBJECT = "/org/queuefocus/QueueFocus"
processes = []


def run(argv, env=None, ok=True):
    result = subprocess.run([str(arg) for arg in argv], env=env, text=True,
                            capture_output=True, timeout=15)
    if ok and result.returncode:
        # Enrollment stdout contains credentials. Never include stdout here.
        raise AssertionError(f"{Path(argv[0]).name} failed: {result.stderr}")
    return result


def wait_for(predicate, label, timeout=18):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError(f"Timed out: {label}")


def launch(argv, **kwargs):
    process = subprocess.Popen([str(arg) for arg in argv], **kwargs)
    processes.append(process)
    return process


def first_line(stream, label):
    if not select.select([stream], [], [], 10)[0]:
        raise AssertionError(f"{label} did not start")
    line = stream.readline().strip()
    assert line, f"{label} exited before startup"
    return line


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


with tempfile.TemporaryDirectory(prefix="qf-sync-native-") as temporary:
    root = Path(temporary)
    logs = []
    succeeded = False
    try:
        database = root / "server/queue.db"
        run([BIN / "qf-sync-server", "init", database])

        def server_start(address):
            process = launch([BIN / "qf-sync-server", "serve", database, address],
                             stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            line = first_line(process.stderr, "sync server")
            match = re.search(r"127\.0\.0\.1:\d+", line)
            assert match, "server did not report its loopback address"
            return process, match[0]

        server, address = server_start("127.0.0.1:0")
        devices = []
        for index, mode in enumerate(["initialize", "shared"]):
            directory = root / f"device-{index}"
            directory.mkdir(mode=0o700)
            runtime = directory / "runtime"
            runtime.mkdir(mode=0o700)
            data = directory / "data/queue-focus"
            enrollment = run([BIN / "qf-sync-server", "enroll", database, f"fixture-{index}"]).stdout
            values = dict(line.split(": ", 1) for line in enrollment.splitlines())
            token = directory / "credential.json"
            token.write_text(json.dumps({"token": values["Token"]}))
            token.chmod(0o600)
            run([BIN / "qf-sync", "setup", data, f"http://{address}",
                 values["Device"], token, mode, "--allow-loopback-http"])
            token.unlink()
            config = directory / "bus.conf"
            config.write_text(f"""<busconfig><type>session</type>
<listen>unix:tmpdir={runtime}</listen><auth>EXTERNAL</auth>
<policy context="default"><allow own="*"/><allow send_destination="*"/>
<allow receive_sender="*"/></policy></busconfig>""")
            bus = launch(["dbus-daemon", "--nofork", f"--config-file={config}", "--print-address=1"],
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            bus_address = first_line(bus.stdout, "private bus")
            env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=bus_address,
                       XDG_DATA_HOME=str(directory / "data"), XDG_RUNTIME_DIR=str(runtime),
                       XDG_CONFIG_HOME=str(directory / "config"), XDG_CACHE_HOME=str(directory / "cache"),
                       GIO_USE_VFS="local", GTK_A11Y="none", GDK_BACKEND="x11",
                       GSK_RENDERER="cairo", GSETTINGS_BACKEND="memory")
            log = (directory / "service.log").open("w+")
            logs.append(log)
            app = launch([BIN / "queue-focus", "service"], env=env, stdout=log, stderr=log)
            devices.append({"env": env, "app": app, "data": data, "log": log})

        def state(device):
            return json.loads(run([BIN / "queue-focus", "status"], device["env"]).stdout)

        def call(device, method, *args, pending=False):
            result = run(["gdbus", "call", "--session", "--dest", NAME, "--object-path", OBJECT,
                          "--method", f"{NAME}1.{method}", *args], device["env"], ok=not pending)
            if pending:
                assert result.returncode and "org.queuefocus.Error.Pending:" in result.stderr, result.stderr
            return result

        a, b = devices
        wait_for(lambda: all(state(d)["sync"]["state"] == "online" for d in devices), "clients online")
        run([BIN / "queue-focus", "add", "Alpha"], a["env"])
        wait_for(lambda: all([t["title"] for t in state(d)["next"]] == ["Alpha"]
                            and state(d)["sync"]["pending"] == 0 for d in devices), "queued capture")
        assert all(state(d)["current"] is None for d in devices), "ordinary add acquired Now"
        call(a, "Promote", str(state(a)["next"][0]["id"]), pending=True)
        wait_for(lambda: all(state(d)["current"] and state(d)["current"]["title"] == "Alpha"
                            and state(d)["sync"]["pending"] == 0 for d in devices), "shared Now")
        call(b, "TogglePause", pending=True)
        wait_for(lambda: all(state(d)["current"]["paused_at"] is not None
                            and state(d)["sync"]["pending"] == 0 for d in devices), "shared pause")
        elapsed = [state(d)["current"]["paused_at"] - state(d)["current"]["started_at"] for d in devices]
        assert abs(elapsed[0] - elapsed[1]) <= 1, elapsed

        stop(server)
        wait_for(lambda: all(state(d)["sync"]["state"] == "offline" for d in devices), "offline status")
        run([BIN / "queue-focus", "add", "Offline capture"], a["env"])
        assert state(a)["sync"]["pending"] == 1
        run([BIN / "queue-focus", "quit"], a["env"])
        a["app"].wait(timeout=5)
        a["app"] = launch([BIN / "queue-focus", "service"], env=a["env"],
                          stdout=a["log"], stderr=a["log"])
        wait_for(lambda: state(a)["sync"]["pending"] == 1, "journal survives app restart")
        refused = run(["gdbus", "call", "--session", "--dest", NAME, "--object-path", OBJECT,
                       "--method", f"{NAME}1.TogglePause"], a["env"], ok=False)
        assert refused.returncode and "wait for sync" in refused.stderr, refused.stderr
        server, _ = server_start(address)
        wait_for(lambda: all(state(d)["sync"]["pending"] == 0
                            and [t["title"] for t in state(d)["next"]] == ["Offline capture"]
                            for d in devices), "offline capture converges without duplication", timeout=40)
        alpha = state(b)["current"]["id"]
        call(b, "CompleteCurrent", pending=True)
        wait_for(lambda: all(state(d)["current"] and state(d)["current"]["title"] == "Offline capture"
                            and state(d)["sync"]["pending"] == 0 for d in devices), "shared completion")
        assert state(b)["sync"]["undo"]["task"] == alpha
        call(b, "UndoComplete", str(alpha), pending=True)
        wait_for(lambda: all(state(d)["current"] and state(d)["current"]["title"] == "Alpha"
                            and state(d)["sync"]["pending"] == 0 for d in devices), "server undo")
        queued = state(a)["next"][0]["id"]
        call(a, "Complete", str(queued))
        assert "false" in call(a, "UndoComplete", str(queued)).stdout, "queued completion offered undo"
        wait_for(lambda: all(not state(d)["next"] and state(d)["sync"]["pending"] == 0
                            for d in devices), "queued deletion")
        for device in devices:
            assert json.loads((device["data"] / "tasks.json").read_text())["tasks"].startswith("Sync is enabled")
        print("native sync integration passed: two primary services, forwarded CLI, shared Now/pause/undo, offline restart and convergence")
        succeeded = True
    finally:
        for process in reversed(processes):
            stop(process)
        for log in logs:
            if not succeeded:
                log.seek(0)
                print(log.read()[-6000:])
            log.close()
