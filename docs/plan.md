# MVP build plan

lyncast is built in the steps below. Each step ends in a check that passes before the next step starts. The [decision log](decisions.md) defines what each step builds. The design docs explain how.

## Steps

1. **Scaffold.** The cargo workspace, `LICENSE`, and these docs exist. Add `scripts/test-env.sh`, which starts Xvfb and smithay `anvil` with their own `XDG_RUNTIME_DIR` ([testing](design/testing.md)). Add a test that fails if `lyncast-core` depends on `gpui`, `wayland-*`, `wl-clipboard-rs`, `rusqlite`, or `niri-ipc` (`cargo tree -p lyncast-core`). Add the bench harness from the summon probe. **Done when** `cargo test` passes and `scripts/test-env.sh` prints a running `WAYLAND_DISPLAY`.
2. **Core shapes.** `Key`, `Row`, `Kind`, `Section`, `Command`, `Subject`, `Chord`, `Verb`, `ActionItem`, `Actions`, and the `actions` table in `lyncast-core` ([architecture](design/architecture.md)). **Done when** unit tests show each row kind's actions and default verb as literal values.
3. **Calculator.** Port the base prototype into `lyncast-core::calc` with the MVP fixes from the decision log ([calculator](design/calculator.md)). **Done when** every fixture in the calculator doc passes, including `12×34`, `1920x1080`, `0x10`, `2026-10-14`, and `123456789012.34+0`.
4. **App index and ranking.** Desktop entry scanning, the `Exec` tokenizer, `frizbee` matching, TinyCast's comparator, and frecency with an injected clock. Built-in commands rank in the same list. **Done when** `lyncast query firef --json` against a fixture `XDG_DATA_DIRS` prints the expected rows byte for byte, and the empty query prints Suggestions then Commands.
5. **Daemon and CLI.** The Unix socket, `--headless`, `Request`, and the CLI commands `toggle`, `query`, and `activate`, with exit codes 0, 1, and 2. Commands run through a runner that tests replace with a recorder. **Done when** Tier 1 tests round-trip each request through a headless daemon, and `activate cmd:reboot` without `--yes` asks for confirmation instead of running.
6. **Clipboard.** The capture thread, the three filters, the `Clips` store, delivery, retention, and the `clip` CLI commands ([clipboard and paste](design/clipboard-and-paste.md)). **Done when** the Tier 2 tests under anvil pass, including a re-copied image read back as a non-empty PNG and an empty clipboard after the daemon stops.
7. **Palette.** The layer-shell window, the input, rows and sections, the calculator card, Suggestions and Commands, the clipboard screen, the Actions menu, close on key release, and cancel on focus loss. **Done when** `TestAppContext` keystroke tests pass, the Tier 2 summon test shows `open` then `closed` around `activate`, and a screenshot under anvil shows the palette.
8. **Launching.** niri IPC `Action Spawn` for apps, plus the system-action commands. **Done when** a fake niri socket records the exact `Spawn` request for a fixture app.
9. **Bench gate.** Summon latency, memory states, growth over 100 cycles, and idle CPU on lavapipe, checked against the [budgets](decisions.md#budgets). Then a real run on Hongyi's AMD niri machine. **Done when** the CI gate passes and the real-hardware numbers are in `docs/research/`.
10. **Install.** A systemd user unit, a niri config snippet (bind and blur `layer-rule`), and install steps in the README. **Done when** a fresh niri session starts the daemon and `Mod+Space` opens the palette.

## After the MVP

These are decided as follow-ups, roughly in this order:

1. Read the freedesktop thumbnail cache for file previews.
2. Aliases and favorites, with a settings surface to edit them.
3. Paste v1 through the niri virtual keyboard ([clipboard and paste](design/clipboard-and-paste.md)).
4. A dot on running apps, from niri IPC.
5. Following the system light or dark setting.
6. Units and currency in the calculator engine.
