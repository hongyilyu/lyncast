# lyncast docs

Start with the [decision log](decisions.md). It is the source of truth, and every other page links back to it.

## Design

- [Architecture](design/architecture.md): the resident process, the pure core, the data shapes, the input box, a summon from toggle to close, and the CLI contract.
- [Calculator](design/calculator.md): the engine that reads the root query, its pipeline and rules, and the fixtures.
- [Daemon](design/daemon.md): starting the daemon, the socket and its wire format, the CLI commands and their output, activation, the runner and its recorder, visits, and the fixtures.
- [Clipboard and paste](design/clipboard-and-paste.md): capture, storage, delivery, the clipboard screen, and the paste seam.
- [Ranking](design/ranking.md): the app index, desktop entries and the Exec tokenizer, the fold, frizbee's weights, the comparator, frecency, the empty list, `lyncast query`, and the fixtures.
- [Testing](design/testing.md): the three test tiers on a headless box, the test environment, and the bench gate.

## Research

- [Landscape](research/landscape.md): existing Linux launchers and why lyncast exists.
- [GPUI on Linux](research/gpui-on-linux.md): what GPUI can do for a launcher, with probe measurements.
- [Wayland and niri](research/wayland-and-niri.md): protocols, IPC, clipboard, launching, matchers, and paste mechanisms.
- [TinyCast spec](research/tinycast-spec.md): the TinyCast behaviours lyncast keeps, adapts, or drops.
- [Raw notes](research/notes/): the agent reports and workflow outputs behind the pages above, kept verbatim.

## Plan

- [MVP build plan](plan.md): the build steps, each with its check, and the follow-ups after the MVP.
