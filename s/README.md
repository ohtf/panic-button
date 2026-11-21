# `s/`

The `s/` directory contains some simple convenience scripts to speed up and standardise working with this repository.

## `s/test`

Run the full test suite, including simulator scenarios and properties.

- `s/test` - everything
- `s/test -p panic-button-sim` - one crate (`panic-button`, `panic-button-sim` or `panic-button-desktop`)
- `PROPTEST_CASES=256 s/test --release -p panic-button-sim --test properties` - CI-scale property run
- `PROPTEST_CASES=1000 s/test --release -p panic-button-sim --test properties` - long property run (about an hour)

## `s/install`

Install from this checkout. The command-line programs go into `~/.cargo/bin`. With `--desktop`, the desktop app is also built and installed as a system package: `panic-button-git` on Arch (via the AUR PKGBUILD), the deb on Debian and Ubuntu, the rpm on Fedora. Run it again to replace the installed version. Asks for your sudo password at the end.

- `s/install` - the command-line programs
- `s/install --desktop` - and the desktop app as a system package (sudo)
- `s/install --user` - and the desktop app for your account only, with a launcher entry "Panic Button (from source)" (no sudo)

## `s/docs`

Serve the documentation site locally with live reload, on the first free port from 8000. First time: `python3 -m venv .venv && ./.venv/bin/pip install -r requirements.txt`.

- `s/docs`

## `s/check-docs-nav`

Fail if a page under `docs/` is missing from the nav in `mkdocs.yml`, or the nav names a missing page. Runs in the docs workflow.

- `s/check-docs-nav`

## `s/version++`

Release: bump the version (`patch`, `minor` or `major`), regenerate `CHANGELOG.md` and push the release commit. CI tags it and publishes to GitHub Releases, Homebrew, the AUR and crates.io.

- `s/version++ minor`

## `s/check-deep`

The slow checks that GitHub Actions no longer runs on every push: simulator properties and scenarios, network-namespace scenarios, the desktop app, and a short fuzz of every target. Run it before pushing changes to `panic-button-core`, `panic-button-net` or the agent.

- `s/check-deep` - about 25 minutes
- `s/check-deep --long` - 1,000 property cases, S13 at 250 devices, 10 minutes per fuzz target (a few hours)

## `s/lint`

Run rustfmt, clippy, cargo-deny and `reuse lint`, as CI does.

- `s/lint`

## `s/netns-test`

Build the release binaries (agent, CLI, relay) and run the network-namespace scenarios (Linux). Runs rootless through `unshare` when not root.

- `s/netns-test` - all scenarios
- `KEEP=1 s/netns-test s7` - one scenario, keeping logs

## `s/gui-dev`

Run the desktop app in development (Vite hot reload plus the Tauri shell).

- `s/gui-dev`
- `PANIC_BUTTON_DATA_DIR=/tmp/a PANIC_BUTTON_CONFIG=/tmp/a.toml s/gui-dev` - a throwaway practice (see `docs/gui/`)

## `s/desktop-check`

Lint (ESLint, Prettier), type-check and build the frontend, then clippy and test the Tauri crate. `npm run format` in `apps/desktop` fixes formatting. `s/test` and `s/lint` leave the desktop crate out because it needs WebKit and a built frontend.

- `s/desktop-check`

## `s/app-icons`

Regenerate the desktop launcher and bundle icons from [the README logo](../docs/assets/panic-button-logo.svg) after installing the desktop npm dependencies. The system tray uses its own flat `tray.png`, which this script leaves alone.

- `s/app-icons`

## `s/x11-test`

Build the release desktop app and CLI, then check them on a real X server in an Arch container (Xvfb, with and without a window manager): the button's size and corner, that a moved button is remembered per monitor layout, and that holding the keyboard shortcut raises an alarm. Needs Docker. Part of `s/check-deep`.

- `s/x11-test`

## `s/gui-test`

Build the release desktop app, then run the GUI run-through (R35) in an Arch container: three copies of the app set up a practice, join it (with a code, and by approval), raise and answer an alarm, run a test alarm, and change rooms, admins and practice settings, all through their windows, driven by WebDriver. Needs Docker and `tauri-driver` (`cargo install tauri-driver --locked`). About 4 minutes.

- `s/gui-test`

## `s/windows-check`

Clippy the panic-button crate and the desktop app for Windows (the GNU target, with MinGW) in a Rust container, so Windows-only code is at least compiled and linted here. Needs Docker and a built frontend. Part of `s/check-deep`.

- `s/windows-check`

## `s/sounds`

Regenerate the built-in sounds in `sounds/` from the synthesiser, then commit them. A test checks the files match.

- `s/sounds`

## `s/fuzz`

Run a cargo-fuzz target (needs nightly and cargo-fuzz).

- `s/fuzz envelope_decode -- -max_total_time=600`

## `s/install-hooks`

Use the tracked hooks in `.githooks/` (pre-commit runs `s/lint`).

- `s/install-hooks`
