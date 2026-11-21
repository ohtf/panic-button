# SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
# SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception
#
# The M5 exit run-through (R35), run inside a container by tests/gui/run.sh:
# a practice is set up, enrolled, drilled and administered entirely from the
# desktop app's windows. Three copies of the real app run side by side on
# loopback, each driven over WebDriver (tauri-driver and WebKitWebDriver) by
# clicking and typing as a person would:
#
#   A sets up the practice, practises the hold and makes an enrolment code.
#   B joins with the code. C asks to join, and A approves it.
#   B holds the button: A says "I'm coming", C "Seen", B resolves.
#   A runs a test alarm; B and C show it; A's report says 2 of 2.
#   A renames C's room, makes B an admin, changes the hold time, removes C.
#
# Only the windows are used to act. Nothing here calls the agent directly.

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

APP = "/w/panic-button-desktop"
PASS = "a long run-through passphrase"
ELEMENT = "element-6066-11e4-a52e-4f735466cecf"
failures = []
T0 = time.time()


def check(name, ok, detail=""):
    print(("ok   " if ok else "FAIL ") + f"[{time.time() - T0:5.0f}s] " + name + (f": {detail}" if detail else ""),
          flush=True)
    if not ok:
        failures.append(name)
    return ok


class Gone(Exception):
    pass


class App:
    """One copy of the desktop app, with its own data, port and driver."""

    def __init__(self, name, n):
        self.name = name
        self.port = 47510 + n
        self.dir = f"/tmp/{name}"
        self.base = f"http://127.0.0.1:{4440 + 2 * n}"
        self.native = 4441 + 2 * n
        self.handles = {}

    def start(self, peers):
        os.makedirs(self.dir, exist_ok=True)
        seeds = ", ".join(f'"127.0.0.1:{p.port}"' for p in peers)
        cfg = f"{self.dir}/config.toml"
        with open(cfg, "w") as f:
            f.write(f"[network]\nport = {self.port}\nmdns = false\nseed_peers = [{seeds}]\n")
        env = dict(
            os.environ,
            DISPLAY=":99",
            PANIC_BUTTON_DATA_DIR=f"{self.dir}/data",
            PANIC_BUTTON_CONFIG=cfg,
            PANIC_BUTTON_CONTROL_SOCKET=f"{self.dir}/control.sock",
            PANIC_BUTTON_LOG="info",
        )
        env.pop("WAYLAND_DISPLAY", None)
        log = open(f"{self.dir}/app.log", "w")
        port = self.base.rsplit(":", 1)[1]
        self.driver = subprocess.Popen(
            ["/w/tauri-driver", "--port", port, "--native-port", str(self.native)],
            env=env, stdout=log, stderr=subprocess.STDOUT,
        )
        for _ in range(50):
            try:
                urllib.request.urlopen(self.base + "/status", timeout=1)
                break
            except (urllib.error.URLError, ConnectionError):
                time.sleep(0.2)
        caps = {"capabilities": {"alwaysMatch": {"tauri:options": {"application": APP}}}}
        self.sid = self.req("POST", "/session", caps, session=False)["sessionId"]
        until(lambda: len(self.req("GET", "/window/handles")) >= 4, 30, f"{self.name}: four windows")
        for h in self.req("GET", "/window/handles"):
            self.req("POST", "/window", {"handle": h})
            view = self.req("GET", "/url").split("view=")[-1]
            self.handles[view] = h

    def stop(self):
        try:
            self.req("DELETE", "")
        except Exception:
            pass
        self.driver.terminate()

    def req(self, method, path, body=None, session=True):
        url = self.base + (f"/session/{self.sid}" if session else "") + path
        data = json.dumps(body if body is not None else {}).encode() if method == "POST" else None
        r = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(r, timeout=60) as f:
                return json.load(f).get("value")
        except urllib.error.HTTPError as e:
            raise Gone(f"{self.name} {method} {path}: {e.read().decode(errors='replace')[:300]}") from None

    def on(self, view):
        self.req("POST", "/window", {"handle": self.handles[view]})
        return self

    def js(self, script, *args):
        return self.req("POST", "/execute/sync", {"script": script, "args": list(args)})

    def text(self):
        return self.js("return document.body.innerText") or ""

    def find(self, xpath):
        els = self.req("POST", "/elements", {"using": "xpath", "value": xpath})
        return els[0][ELEMENT] if els else None

    def click(self, xpath, what):
        until(lambda: self.find(xpath), 20, f"{self.name}: {what} to click")
        e = self.find(xpath)
        self.req("POST", f"/element/{e}/click")

    def button(self, label):
        t = lit(label)
        self.click(f"//button[normalize-space(.)={t} or normalize-space(strong)={t}]", f'"{label}"')

    def type(self, css, value):
        until(lambda: self.req("POST", "/elements", {"using": "css selector", "value": css}), 20,
              f"{self.name}: {css}")
        e = self.req("POST", "/element", {"using": "css selector", "value": css})[ELEMENT]
        self.req("POST", f"/element/{e}/clear")
        self.req("POST", f"/element/{e}/value", {"text": value})

    def select(self, css, label):
        self.click(f"//select[@id='{css}']/option[normalize-space(.)='{label}']", f"{label} in #{css}")

    def hold(self, css, ms):
        """Presses and holds with the mouse, as a person does."""
        if not until(lambda: self.req("POST", "/elements", {"using": "css selector", "value": css}), 20, None):
            html = self.js("return document.body.innerHTML")[:600]
            raise Gone(f"{self.name}: no {css} in {html}")
        e = self.req("POST", "/element", {"using": "css selector", "value": css})[ELEMENT]
        self.req("POST", "/actions", {"actions": [{
            "type": "pointer", "id": "mouse", "parameters": {"pointerType": "mouse"},
            "actions": [
                {"type": "pointerMove", "origin": {ELEMENT: e}, "x": 0, "y": 0},
                {"type": "pointerDown", "button": 0},
                {"type": "pause", "duration": ms},
                {"type": "pointerUp", "button": 0},
            ]}]})
        self.req("DELETE", "/actions")

    def wait_text(self, view, needle, secs=30):
        """Waits until the window shows `needle`; reports it as a check."""
        self.on(view)
        ok = until(lambda: needle in self.on(view).text(), secs, None)
        return check(f"{self.name} {view} shows {needle!r}", ok,
                     "" if ok else self.on(view).text()[:300].replace("\n", " | "))


def lit(s):
    """An XPath string literal."""
    return f"'{s}'" if "'" not in s else f'"{s}"'


def until(f, secs, what):
    end = time.time() + secs
    while time.time() < end:
        try:
            if f():
                return True
        except Gone:
            pass
        time.sleep(0.5)
    if what:
        raise Gone(f"timed out waiting for {what}")
    return False


def tab(app, name):
    app.on("main").click(f"//nav//button[normalize-space(.)='{name}']", f"{name} tab")


def row(room, label):
    return f"//tr[td[contains(normalize-space(.), '{room}')]]//button[normalize-space(.)='{label}']"


def main():
    a, b, c = App("a", 1), App("b", 2), App("c", 3)
    apps = [a, b, c]
    try:
        run(a, b, c)
    except Gone as e:
        check("run-through finished", False, str(e))
    finally:
        for x in apps:
            x.stop()
        if failures:
            for x in apps:
                print(f"--- {x.name} log (last 25 lines)")
                with open(f"{x.dir}/app.log", errors="replace") as f:
                    lines = [ln for ln in f if "audio stream error" not in ln]
                    print("".join(lines[-25:]))
    if failures:
        print("FAILURES: " + ", ".join(failures))
        sys.exit(1)
    print("all passed")


def run(a, b, c):
    # A: set up the practice, choose the room, practise the hold.
    a.start([b, c])
    a.wait_text("main", "Welcome to Panic Button")
    a.on("main").button("Set up a new practice")
    a.type("#pname", "Riverside Surgery")
    a.type("#plabel", "Practice Manager")
    a.type("#pass", PASS)
    a.type("#pass2", PASS)
    a.type("#kit", "/tmp/a/recovery.pbkit")
    a.button("Create practice")
    a.wait_text("main", "Riverside Surgery is ready", 60)
    check("A recovery kit written", os.path.exists("/tmp/a/recovery.pbkit"))
    a.button("Next")
    a.type("#room", "Reception")
    a.button("Next")
    a.wait_text("main", "Try it now")
    a.hold("button[aria-label='Practice: press and hold']", 2500)
    a.wait_text("main", "Done. Holding for the full time")
    a.button("Finish")
    a.wait_text("main", "Devices in your practice")

    # A: an enrolment code from the Admin tab.
    tab(a, "Admin")
    a.wait_text("main", "Add a computer")
    a.type("#tlabel", "Consulting rooms")
    a.button("Create code")
    until(lambda: a.on("main").js(
        "const t = document.querySelector(\"textarea[aria-label='Enrolment code']\"); return t && t.value"),
        20, "A: the enrolment code")
    code = a.js("return document.querySelector(\"textarea[aria-label='Enrolment code']\").value")
    check("A made an enrolment code", code.startswith("panic-button-enrol-1:"), code[:30])

    # B: join with the code.
    b.start([a, c])
    b.wait_text("main", "Welcome to Panic Button")
    b.wait_text("main", "Panic Button is running on 1 other computer")
    b.on("main").button("Join an existing practice")
    b.type("#token", code)
    b.button("Join")
    b.wait_text("main", "Which room is this computer in?", 60)
    b.type("#room", "Consulting Room 4")
    b.button("Next")
    b.wait_text("main", "Try it now")
    b.button("Skip")
    b.wait_text("main", "Devices in your practice")

    # C: ask to join; A approves after checking the code.
    c.start([a, b])
    c.wait_text("main", "Panic Button is running on 2 other computers", 60)
    c.on("main").button("Ask to join")
    until(lambda: c.on("main").js("const e = document.querySelector('.join-code'); return e && e.textContent"),
          30, "C: its join code")
    join_code = "".join(ch for ch in c.js("return document.querySelector('.join-code').textContent")
                        if ch.isdigit())
    check("C shows a six-digit code", len(join_code) == 6, join_code)
    tab(a, "Admin")
    a.wait_text("main", "Waiting to join", 60)
    shown = a.on("main").js("const e = document.querySelector('.join .code'); return e && e.textContent") or ""
    check("A shows the same code", "".join(ch for ch in shown if ch.isdigit()) == join_code, shown)
    a.button("Approve")
    c.wait_text("main", "Join Riverside Surgery?", 60)
    c.on("main").button("Join Riverside Surgery")
    c.wait_text("main", "Which room is this computer in?", 60)
    c.type("#room", "Consulting Room 5")
    c.button("Next")
    c.wait_text("main", "Try it now")
    c.button("Skip")

    # Everyone sees everyone.
    for x, others in ((a, ["Consulting Room 4", "Consulting Room 5"]),
                      (b, ["Reception", "Consulting Room 5"]),
                      (c, ["Reception", "Consulting Room 4"])):
        tab(x, "Status")
        for room in others:
            x.wait_text("main", room, 60)
        x.wait_text("main", "Connected to 2 devices", 60)

    # A real call for help from B, by holding its button.
    b.on("button").hold("button[aria-label^='Press and hold to call for help']", 2500)
    for x in (a, c):
        x.wait_text("alert", "HELP NEEDED", 15)
        x.wait_text("alert", "Consulting Room 4", 5)
    b.wait_text("panel", "Delivered to 2 devices", 15)
    a.on("alert").button("I'm coming")
    c.on("alert").button("Seen - can't come")
    b.wait_text("panel", "Coming:", 15)
    b.wait_text("panel", "Practice Manager", 5)  # A's name from setup
    b.on("panel").button("Resolved")
    b.on("panel").button("Resolve")
    b.wait_text("panel", "Resolved", 10)
    for x in (a, c):
        x.wait_text("alert", "Resolved", 15)
    tab(a, "History")
    a.wait_text("main", "Consulting Room 4", 10)

    # A test alarm from A's Admin tab.
    tab(a, "Admin")
    a.on("main").button("Run a test alarm now")
    # The notice shows for 10 seconds, so watch both at once.
    seen = set()
    until(lambda: not seen.update(x.name for x in (b, c) if "Test alarm received" in x.on("panel").text())
          and len(seen) == 2, 20, None)
    for x in (b, c):
        check(f"{x.name} panel shows 'Test alarm received'", x.name in seen)
    a.wait_text("main", "2 of 2 computers", 120)

    # Administration from A: C's room, B made an admin, the hold time, C removed.
    a.on("main").button("All")
    a.click(row("Consulting Room 5", "Edit"), "Edit on C's row")
    a.type("input[aria-label='Room']", "Consulting Room 6")
    a.button("Save")
    # 'Changing to "Consulting Room 6"' until C confirms, which may be at once.
    a.wait_text("main", "Consulting Room 6", 10)
    tab(c, "Settings")
    ok = until(lambda: c.on("main").js("return document.getElementById('room').value") == "Consulting Room 6",
               30, None)
    check("C's own Settings show the room A gave it", ok)
    tab(a, "Admin")
    ok = until(lambda: "Changing to" not in a.on("main").text() and "Consulting Room 6" in a.on("main").text(),
               60, None)
    check("A's devices show C confirmed the room", ok)

    tab(a, "Admin")
    a.wait_text("main", "This practice has only one Practice Admin")
    a.click(row("Consulting Room 4", "Make admin..."), "Make admin on B's row")
    a.button("Make admin")
    ok = until(lambda: "only one Practice Admin" not in a.on("main").text(), 30, None)
    check("A no longer warns about a single admin", ok)
    ok = until(lambda: b.on("main").find("//nav//button[normalize-space(.)='Admin']"), 30, None)
    check("B now has the Admin tab", bool(ok))

    a.on("main").select("s-hold", "2 seconds")
    a.button("Save practice settings")
    tab(b, "Admin")
    ok = until(lambda: b.on("main").js("return document.getElementById('s-hold').selectedOptions[0].text")
               == "2 seconds", 30, None)
    check("B received the new hold time", ok)

    tab(a, "Admin")
    a.on("main").button("All")
    a.click(row("Consulting Room 6", "Remove..."), "Remove on C's row")
    a.button("Remove")
    tab(c, "Status")
    ok = until(lambda: "removed" in c.on("main").text().lower(), 60, None)
    check("C says it was removed from the practice", ok, "" if ok else c.on("main").text()[:300])
    ok = until(lambda: "Consulting Room 6" not in a.on("main").text(), 30, None)
    check("C is gone from A's devices", ok)


if __name__ == "__main__":
    main()
