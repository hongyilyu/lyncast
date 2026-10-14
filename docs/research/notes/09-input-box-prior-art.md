# How GPUI launchers handle the input box (lead investigation, 2026-10-14)

Read from source in local clones: zlaunch at commit `dac7173` (2026-06-19) and look at its main branch as of 2026-10-14. Paths are relative to each repo.

## zlaunch

On every keystroke, `src/ui/delegates/dynamic_items.rs:34-92` does the following:

1. Fuzzy-filters apps, windows, and commands with `fuzzy-matcher` 0.3 `SkimMatcherV2`. `src/ui/delegates/item_filter.rs` adds bonuses for an exact name, a name prefix, a word prefix, and contiguous matches. It demotes description-only matches. A query with spaces is retried without spaces and with hyphens ("counter strike" matches "counterstrike").
2. Runs the calculator only if the query contains a digit. `src/calculator/evaluation.rs` calls `fend_core::evaluate_with_interrupt` with a 50 ms timeout. A parse makes a Calculator row. Division by zero shows "Infinity".
3. Checks explicit prefixes, first match wins. `!ai` shows only the AI row. `!g`, `!ddg`, and the other search triggers show only that provider (`src/search/detection.rs`).
4. Otherwise adds an AI row and every web-search provider as fallbacks.

`src/ui/delegates/section_manager.rs` orders sections: Best Match, Calculator, Windows, Commands, Applications, Search and AI.

## look

Prefix scopes come from one table, `MODES` in `core/engine/src/modes.rs`, parsed in `core/engine/src/query.rs`. Examples: `c"` clipboard, `a"` apps, `f"` files, `d"` folders, `r"` regex, `rc"` recent, `ps"` processes. A scoped query skips URL and calculator rows.

A plain query, in `apps/linows/gpui/src/query.rs:93-127`:

1. Runs the fuzzy search over apps and files with its own `look-matching` crate (a DP scorer: match base 10, word-boundary bonus 8, leading bonus over the first 5 characters, consecutive bonus 6, gap penalty capped at 20, query capped at 64 characters, Unicode folded first).
2. Prepends quick folders, classifies URLs, and merges recent URLs by score.
3. Inserts the calculator answer at index 0 when `calc_inline` returns one. The code comment says: "an expression is a question, and the answer outranks a file that fuzzy-matched some of its digits."

The calculator gate `is_math` is guarded by a corpus of real launcher queries that must not be math (`core/calc/tests/intent.rs`): dates (`2026-08-04`, `20-05-2026`), versions and addresses (`1.2.3`, `192.168.1.1`), resolutions and ratios (`1920x1080`, `16:9`), and names with numbers (`covid-19`, `IMG_20240115`). Its rule: "shape decides, spacing never does".

## Other matchers seen

- Zed, where GPUI comes from, uses its own `fuzzy` crate (character-bag prefilter) and `fuzzy_nucleo` for the command palette.
- Loungy uses `nucleo` 0.3.

## Conclusion

Both launchers route explicit prefixes to one feature and run every feature on a plain query, then merge with the calculator answer on top. Neither picks a feature by sniffing keywords.
