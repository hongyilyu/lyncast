# TinyCast behaviour spec

TinyCast (`abue-ammar/tinycast`) is a macOS launcher licensed AGPL-3.0. lyncast ports none of TinyCast's code ([decisions](../decisions.md#project)). The notes read each fact in TinyCast's docs unless marked inferred. If this page disagrees with [decisions.md](../decisions.md), the decision log wins.

## The palette steps back one Escape at a time over one flat index

[notes/06](notes/06-tinycast-behaviour-spec.md#1-palette-and-navigation) records these rules:

- Option+Space toggles the palette.
- Loss of key focus hides the palette.
- Escape clears the query, then pops a pushed screen, then hides. Backspace in an empty field takes the same steps but never closes.
- A summon inside `popToRootTimeout` after a hide keeps the state.
- Tab cycles launcher, AI, and clipboard, and carries the query into clipboard.
- Selection is one flat index in visible row order, and headers take no index. Each screen lands on row 0, except clipboard, which lands on the newest clip below the pins.
- The field never submits. Pasted line breaks become spaces. Ctrl+N and Ctrl+P act as Down and Up.
- A typed query merges the calculator card at index 0, the colour card, "Open in Browser" for URLs, ranked matches, category sections, and fallbacks, in that order.

## The launcher ranks with a point scorer, a 14-rule comparator, and frecency

[notes/06](notes/06-tinycast-behaviour-spec.md#2-launcher) records these rules.

The scorer matches the title, alternate titles, and subtitle. Keywords make an entry appear but never rank it. Points are 4 for the first character on the first character, 3 for a word start, 2 for any other match, 1 for a mismatched separator, and -1 for a gap. Word starts include camelCase and a letter after a digit. An exact match beats every score.

Sensitivity sets the minimum score for a query of length L. Medium is the default and needs `score ≥ 1.5·(L−2)+4`. High needs `score > 2·L`. The fold is NFC, then case, diacritic, and width folding, once per field and once per query.

The comparator applies 14 rules in order. The first rule that separates two entries decides.

1. Exact alias
2. Boosted term
3. Exact title, past 3 characters
4. Exact past search term
5. Exact subtitle
6. Alias prefix
7. Past-term prefix
8. Past term run at most 3 characters past
9. Best field score
10. Frecency
11. The title's own score
12. Title prefix
13. Kind priority, with apps 4, commands 3, quicklinks 2, and panes 1
14. Name, compared numerically

Favorites are an ordered array, shown first only for an empty query.

Frecency decays with time:

```
score(t) = max(1, e^(λ·(anchor − t)))    with λ = ln 2 per 10 days
```

A visit adds 100. The last 3 distinct search terms steer ranking only while the score is above 1 and the entry was opened in the last 17 days.

## Clipboard history stores three types and skips its own writes

[notes/06](notes/06-tinycast-behaviour-spec.md#3-clipboard-history) records these rules:

- TinyCast stores `text`, `image`, and `file` only. It derives link, email, and colour on read.
- Capture polls every 0.5 s and rejects files under temp roots.
- Every write the app makes carries a private marker type, and the poller skips it.
- Password-manager copies and copies from "Disabled Applications" are not recorded.
- Memory holds the newest 1000 unpinned rows. Only age caps the table. Search returns at most 200 rows and uses SQLite FTS5 trigram for 3 or more characters.
- A paste moves a row to the top. Pins keep pin order, survive Clear History, and do not move to the top on paste.
- Storage is `clipboard.sqlite3` plus loose PNG files. File entries point at the original path.

The docs give no retention presets, size cap, or capture-time dedupe rule.

## The calculator has one parser and shows errors only for definite mistakes

[notes/08](notes/08-calc-engine-workflow.md#grounding-tinycast-calculator-spec) records the pipeline:

1. The query becomes canonical English numbers. Answers localize only for display.
2. A single ASCII word returns at once, except `now`, `today`, `tomorrow`, `yesterday`, and `time`. A lone plain number earns no card, but `1e5` does.
3. Before tokenizing, dates and times parse, then time zones. Another passage puts zones last, so the docs conflict.
4. `CalcTokenizer` is the only tokenizer. Stages after it handle trailing operators, bases, typed arithmetic, unit and currency conversion, bare units, and percent forms.
5. `CalcExpressionParser` is the only precedence-climbing evaluator. `CalcOperator` owns identity, binding power, and spelling.

The note adds these rules:

- A trailing operator shows the valid prefix, so `10 +` shows `10`. Only operators count, so `10 of` stays a search.
- Only a definite mistake, such as `1kg + 1m`, shows an error. Everything else is silent, and overflow is rejected. Division by zero is undocumented.
- `x` multiplies only after an operand, which spares `xor`, `max`, and `0x`.
- Juxtaposition multiplies at the binding power of `*`, so `6/2(1+2)` equals `6/2*(1+2)`. Adjacent numbers never multiply, so `5 3` is a search.

[notes/06](notes/06-tinycast-behaviour-spec.md#4-calculator) lists inputs that must stay silent, including `10 of`, `1.2.3 + 1`, and `1e400`.

### A result has an echo, an answer, a copy text, and badges

[notes/08](notes/08-calc-engine-workflow.md#grounding-tinycast-calculator-spec) records the result model:

- `CalcResult` holds `expression`, `display`, `copyText`, and optional `sourceBadge` and `targetBadge` pills. `CalculatorCard` renders two columns, with `expression` on the left.
- `Payload.number` rounds once and then groups digits, as in `10,500 g`. Examples show 10 significant digits, but that rule is inferred.
- A `px` answer copies without the space, so copy text can differ from display.
- Percent forms badge the number as `Sum`, `Total`, or `Average` instead of `Result`.
- No doc states rules for the `Product` badge, the number-in-words badge, or the `×` echo of `x`. Only a screenshot shows them.
- The card is flat index 0. Enter copies the answer and records it to history. Cmd+Enter makes the answer the query, except for date, time, and boolean answers.
- The engine runs synchronously on every keystroke behind a one-deep memo.

## TinyCast pastes on Enter and copies on Cmd+Enter

[notes/07](notes/07-paste-seam-workflow.md#grounding-tinycast-paste-and-copy-contract) records this contract:

- Enter writes the entry, activates `previousApp`, posts a synthetic Cmd+V, and promotes the row.
- Cmd+Enter copies and promotes. The docs do not say whether it closes the palette.
- Ctrl+Cmd+Enter pastes plain text. Option+Enter calls `pasteInPlace`, which posts Cmd+V to the target pid 50 ms after the write, with no activation and no promote.
- `show()` resolves `PasteTarget` once per summon.
- A history paste does not restore the old clipboard. Pinned rows never promote.
- Every write stamps `internalType`, and the poller skips it. If the writer and the poller disagree on the marker, pastes are captured again in a loop.
- For the calculator, TinyCast's `calculator.md` says Enter copies, but its `testing.md` says Enter "pastes 3,8". The two docs conflict.

## TinyCast budgets memory and leaks but not latency

[notes/06](notes/06-tinycast-behaviour-spec.md#5-budgets-and-standards) and [§6](notes/06-tinycast-behaviour-spec.md#6-architecture-worth-borrowing) record these rules:

- "Resident memory under 100 MB, always." Memory returns to baseline after the palette closes.
- "Zero leaks and no retain cycles." Every long-lived task has an owner and a teardown.
- No frame budget or millisecond target exists. Launch time and RSS have never been recorded as numbers.
- Code sits in the layers PURE, EFFECT, OBSERVABLE STATE, and VIEW. A mechanical check keeps UI, clock, network, and filesystem out of PURE.
- There are no XCTest or UI tests. Standalone harnesses compile the shipped pure sources, so a compile failure means a layering leak. Harnesses never touch the real pasteboard.
- A ranking complaint becomes a new fuzz case.

## Each TinyCast item is kept, adapted, deferred, or dropped

The verdicts start from [notes/06](notes/06-tinycast-behaviour-spec.md#7-port-to-linuxniri).

| TinyCast item | Verdict | How in lyncast |
|---|---|---|
| Option+Space hotkey | Adapt | A niri bind runs `lyncast toggle` ([process model](../decisions.md#process-model)). |
| Panel and focus loss | Adapt | An Overlay layer-shell surface with exclusive keyboard focus. Focus loss cancels the summon. |
| Escape steps, flat selection, landing row, one-line query, Ctrl+N and Ctrl+P, IME | Keep | |
| Tab ring | Adapt | Tab or a registered prefix scopes the query ([input box](../decisions.md#input-box)). |
| Root merge order | Adapt | Answer, then Results, where apps and commands rank together. A Fallback section can come later, and the MVP has none ([input box](../decisions.md#input-box)). |
| App index | Adapt | `.desktop` files through lyncast's own parser, launched through niri IPC `Action Spawn` ([app index and ranking](../decisions.md#app-index-and-ranking), [stack](../decisions.md#stack)). |
| Scorer | Adapt | `frizbee` matches with TinyCast's points as its weights, and TinyCast's ranking rules sit on top ([ranking](../design/ranking.md)). |
| Comparator, frecency, aliases, favorites | Keep | The MVP drops rules 1, 2, and 6 until aliases arrive. |
| Fold | Adapt | TinyCast's width, format, diacritic, and case fold, through NFD and the canonical combining class. Half-width katakana, the full-width signs U+FFE0 to U+FFE6, ligatures, and `ß` are not folded ([ranking](../design/ranking.md#one-fold-runs-on-every-field-and-every-query)). |
| Romanization, running dot, drag-out | Drop for MVP | |
| Capture, marker, sensitive guard | Adapt | The `wl-clipboard-rs` `Watcher` skips `application/x-lyncast-own` and `x-kde-passwordManagerHint` ([clipboard and paste](../decisions.md#clipboard-and-paste)). |
| Disabled apps | Defer | data-control does not expose the source app. |
| Storage, pins, limits | Adapt | `rusqlite`, with blobs as files named by hash. Unpinned entries expire after 3 months by default, and entries over 25 MB are not recorded. |
| Clipboard screen | Keep | Type filter, day groups, preview pane, and information panel ([clipboard and paste](../decisions.md#clipboard-and-paste)). |
| Paste-back | Adapt | Copy only in the MVP ([scope](../decisions.md#scope-of-the-mvp)). Synthetic Ctrl+V is unverified on niri ([notes/07](notes/07-paste-seam-workflow.md#grounding-tinycast-paste-and-copy-contract)). |
| Calculator engine | Keep | A pure `calc::evaluate(text, &Env) -> Outcome` with `+`, `-`, `*`, and `/`. |
| Units, currency, dates, time zones | Defer | Units and currency come later as new kinds of quantity. Date-shaped input stays silent ([calculator engine](../decisions.md#calculator-engine)). |
| Memory and leak rules | Keep | Numeric limits ([budgets](../decisions.md#budgets)). |
| Test harnesses | Adapt | CLI integration tests in three tiers ([testing](../decisions.md#testing)). |
| Input-source switching, window switcher, OCR, colours, Paste Sequentially | Drop | |
