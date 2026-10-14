# Calculator engine bake-off workflow (8 agents, 2026-10-14)

Raw workflow outputs, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.


## Grounding: TinyCast calculator spec

Labels: R = read-in-source (TinyCast docs). I = inferred. I measured nothing. Paths under /tmp/tinycast-docs/: calc = calculator.md, pal = palette.md, lau = launcher.md, test = testing.md, arch = architecture.md.

1. Pipeline
- The query is rewritten to canonical English numbers before anything else. Answers localize only when shown (calc:16-20, 634). R
- Single ASCII words return at once. Exceptions: now, today, tomorrow, yesterday, time (calc:48-49, 129-131). R
- A lone plain number earns no card. `1e5` does, because it is shorthand (calc:299-300). R
- Before tokenizing: date/time, then time zones (calc:51-55). The doc also says zones run last, after units and currency (calc:389-391). These two statements conflict. R
- Then comes one tokenizer, `CalcTokenizer` (calc:69). Stages after it: trailing-operator prefix, base conversion, typed arithmetic, explicit unit conversion, currency conversion, bare-unit auto-conversion, percent forms (calc:55-62). R
- `CalcExpressionParser` is the only precedence-climbing evaluator. It returns `CalcValue` (number, measurement, currency, boolean). There is no second parser and no fallback (calc:64-67). `CalcOperator` owns identity, binding power and spelling (calc:70). R
- Typed arithmetic runs before conversion, so `1 / 20ms to hz` divides by a duration (calc:85-86). R
- Trailing operator: `10 +` shows `10`. The prefix must be valid. Open parens stay silent. Only operators count; `10 of` stays a search. The card echoes the typed text and keeps the prefix's target badge (calc:472-479). Evaluation uses full precision. Display rounding never feeds back (calc:77-78). R
- Silence: an error shows only for input that can only be a mistake, such as `1kg + 1m` or a unit against a currency. Everything else stays silent (calc:270-272). Error texts: `Cannot convert Currency to Weight.` (calc:543-544) and `No exchange rate for <CODE>.` (calc:602). Overflow is rejected (calc:76). `1e400` is not calculator input (calc:300-301). Division by zero is not documented. R

2. Units and currency
- `CalcDimension` holds exponents for length, mass, time, data, current, pixel and currency. Products add them, division subtracts, powers multiply. `baseUnits` maps results back to ordinary units (calc:212-216). R
- A new dimension needs a signature on `UnitCategory`, records in `CalcUnitCatalog` and one output in `baseUnits`. A new unit is one table entry (calc:225-227, 80-83). R
- The last unit typed decides the answer's unit (calc:192-195). A bare number takes its neighbour's unit (calc:204). R
- Currencies join the same `byName` table, so the tokenizer, conversion and arithmetic work unchanged (calc:535-537). Currency runs after units (calc:540-544). R
- `evaluate` never fetches. It takes a finished `CurrencyRates?`, where nil means no snapshot yet (calc:21-23, arch:80-81). Rates are one flat units-per-base map (calc:581-584). The memo keys on `fetchedAt` and region (calc:604-605). R
- For lyncast: model a scalar now as a quantity with zero exponents, and pass `rates: Option<&Rates>` as None. Units then add table rows, not parser code. Once currency lands, the answer depends on the query plus the snapshot. CLI activation must then name or reuse that snapshot. I

3. Result model
- `CalcResult` has `expression` (left), `display` and `copyText` (right), and optional `sourceBadge` and `targetBadge` pills. `CalculatorCard` renders two columns (calc:613-614). R
- `Payload.number` rounds once, then groups the text for display (calc:615). Examples show 10 significant digits (`333.3333333`, calc:218; `163.1783089`, calc:256) and comma grouping (`10,500 g`, calc:194; `100,000`, calc:297). The 10-digit rule itself is I. Money uses 2 decimals, and 4 significant digits below a cent, in plain notation (calc:607-609). R
- Copy vs display: `px` copies without the space (calc:262-263). Zone answers copy the bare time (calc:328-329). The docs do not say whether plain numbers copy with grouping.
- Badges: percent forms name what the number is (`Sum`, `Total`, `Average`) instead of a bare `Result` (calc:461-463). Currency badges read `US Dollar → Bangladeshi Taka` (calc:556). R
- None of the nine docs has the `Product` badge, the number-in-words badge, or the rule that echoes `x` as `×`. The screenshot is the only evidence. No rule covers decimals, negatives or large numbers. Proposal: name the top-level operator (Sum, Difference, Product, Quotient, Square Root). Spell integers in title-case short scale, with "Minus" for negatives. Drop the badge for non-integers or past a length cap. Pin each rule with a CLI test. I

4. Integration
- The card sits at flat index 0 and shifts the other rows by one. Selection matches the visible order (pal:13-16, 235-237; calc:618-619). R
- Enter copies the answer and records it to history. ⌘↵ records it, then makes the answer the query with the caret after it. Only `canChain` answers offer this. Date, time and boolean answers clear the flag (calc:619-623, test:405-407). R
- Rows below the card keep launcher ranking (lau:155-163). I
- The calculator runs synchronously on every keystroke behind a one-deep memo (calc:3-4, 362-363). A ~150 ms network lookup was rejected for that reason (calc:360-365). No numeric latency budget exists. The benchmark only measures timing (test:274-282). R

5. Spellings
- A lone `x` between operands, and `×` (calc:431-432). `x` folds only after an operand, which spares `xof`, `xrp`, `xor`, `max` and `0x` (calc:437-439). R
- `sqrt(...)` (calc:242). `square root of` and `cube root of` (calc:71-72, 435). R
- `÷`, `·`, `−` (U+2212) and `√` are not documented.
- `mod` is spelled out, `%` is percent and `^` is power (calc:467-470, 688). R
- Juxtaposition is `*` at the same binding power: `4(2+3)`, `2pi`, `2sqrt(9)`, `(2+3)(2+3)`. `6/2(1+2)` equals `6/2*(1+2)` (calc:429-431). Parens, constants, functions and spoken roots start a product. Adjacent numbers never do, so `5 3` is a search (calc:435-436). R

6. MVP subset
Applies now (I, each citing the source rule):
- The single-word rule and lone-number silence (calc:48, 300). `5 3` stays silent (calc:436).
- One tokenizer, one precedence parser, and one operator table that owns spelling and binding power (calc:64-70).
- `+ - * /`, `x`, `×`, unary minus, parentheses, `sqrt(...)` and `square root of`.
- Implicit multiplication with parens and roots (calc:429-435).
- The trailing-operator prefix (calc:472-476).
- The silence policy, overflow rejection and full-precision evaluation (calc:76-78, 270-272).
- Round once, group, and keep display separate from copy (calc:615).
- Card at index 0. Enter copies (calc:618-620).
- The expression echo and both badges, once lyncast fixes the section 3 rules.

Waits for units and currency: `CalcDimension`, the unit catalog, `to`/`in` conversion, bare-unit auto-conversion, the last-unit rule, the rate snapshot, region currency, currency errors, and the `k` suffix and magnitude words (calc:274-282). Also deferred: dates, zones, percent, mod, functions, comparisons, bases, locale formats, history and ⌘↵ chaining.

## Grounding: Rust calculator prior art

Roots: F = /tmp/research-wl/fend/core/src, N = /tmp/research-wl/numbat/numbat/src, R = github.com/tiffany352/rink-rs (copies in /tmp/calc-proto/rink). Measurements come from /tmp/calc-proto/measure, built against fend-core 1.5.8 and numbat 1.24.0, release build, on this host. Tags: [M] measured, [S] read in source or docs, [I] inferred.

**1. Pipelines**
- fend: the lexer emits typed tokens with numbers already parsed (F/lexer.rs:10-16). The parser is hand-written recursive descent that backtracks over a token slice, returning `Result<(Expr, &[Token])>` (F/parser.rs:51, 313-339). A tree-walking evaluator runs over `Expr` (F/ast.rs:108-131, 394). [S]
- numbat: tokenizer (N/tokenizer.rs:883), then recursive descent against a documented grammar (N/parser.rs:1-60), then a prefix transformer, a static dimension typechecker, and a bytecode VM (N/lib.rs:710-745; N/vm.rs:297). [S]
- rink: tokenizer, then hand-written recursive descent, then evaluation (R ARCHITECTURE.md:42-45). [S]
- All three put the unit inside the numeric value. fend: `Value{value, unit, exact, ..}` (F/num/unit.rs:36-44). numbat: `Quantity{value, unit}`, where the unit is a product of (prefix, unit, rational exponent) (N/quantity.rs:26-29; N/unit.rs:170-174, 224). rink: `Number{value, unit: Dimensionality}`, reduced to base units at once (R core/src/types/number.rs:17-20; ARCHITECTURE.md:63-70). [S]
- Unit registries:
  - fend: const Rust tuple tables. Each definition is an expression string that the same evaluator runs lazily (F/units/builtin.rs:10-37; F/units.rs:33-120). [S]
  - numbat: `.nbt` prelude source files embedded with rust-embed, then parsed and typechecked at startup (N/../modules/units/si.nbt:7-10; N/module_importer.rs:8, 98). [S]
  - rink: a GNU-units-format text file (R core/definitions.units). [S]

**2. Currency injection**
- fend: the host implements `ExchangeRateFnV2::relative_to_base_currency(code, opts) -> f64` (F/lib.rs:220-238). A currency is a unit equal to `(1/rate) BASE_CURRENCY`, and `BASE_CURRENCY` is a base unit (F/units.rs:41-66; F/units/builtin.rs:765, 886-899). The core does no IO. `is_preview()` tells the handler to answer from cache only (F/lib.rs:226-231). [S]
- Without a handler, fend errors "exchange rates are not available". With a fixed table, "10 USD to EUR" gives "9 EUR". [M]
- numbat: a process-global `OnceLock` is filled by a blocking HTTP fetch from the ECB (N/currency.rs:11, 39-46). A missing rate becomes NaN (N/ffi/currency.rs:20). The currency module loads only after an UnknownIdentifier type error (N/lib.rs:756-800). This is not pure. [S]
- rink: the host downloads JSON and passes the string to `ctx.load_currency(Some(&data), defs)`. Passing `None` makes currency queries return `MissingDeps` (R core/src/lib.rs:37-68). This is the purest of the three. [S]

**3. Staying silent and interrupts**
- None of the engines stays silent. Silence is a policy the host applies. fend's `evaluate_preview_with_interrupt` returns nothing when evaluation errors, when output is over 50 chars or has control chars, or when output equals input (F/lib.rs:607-634). [S]
- In fend's preview, "12", "12*", "1/0" and "firefox" all show nothing. "(1+2" and "1+2)" both preview 3, because the parser repairs parentheses (F/parser.rs:113-117; F/eval.rs:17-28). [M]
- The preview runs on a cloned context, so the `_`/`ans` writes do not leak (F/eval.rs:84-85; F/lib.rs:612-618). [S]
- fend polls an `Interrupt` trait during evaluation (F/interrupt.rs:3-12). `100000!` with a 50 ms deadline returned "interrupted" after 51 ms. [S][M]
- numbat has no interrupt API (grep of N finds none). It errors on "(1+2" with "Missing closing parenthesis". [S][M]
- rink's frontends run evaluation in a separate sandbox crate that limits time and memory (R sandbox/src/lib.rs:5-7). [S]

**4. Number representation**
- fend: its own BigUint and BigRat, a pi-multiple variant and an exact flag (F/num/bigrat.rs:64-68; F/num/real.rs:37-42; F/num/exact.rs:7-10). sqrt is exact for perfect squares. Otherwise it uses Newton iteration and marks the result approximate (F/num/bigrat.rs:1068-1114). Trig goes through f64 (F/num/bigrat.rs:252-327). Inexact results print to 10 decimal places with "approx." (F/num/formatting_style.rs:24-27). [S]
- fend results: `0.1+0.2==0.3` is true. `10^100+1-10^100` is 1. `1/3` prints "approx. 0.3333333333". One evaluation takes 6 to 92 µs. [M]
- numbat: `Number(pub f64)`, with a TODO on that line (N/number.rs:9-10). Display uses 6 significant digits (N/pretty_print.rs:26). [S]
- numbat results: `10^100+1-10^100` is 0. `0.1+0.2==0.3` is false. `sqrt(2)*sqrt(2)` prints "2.0". Loading the prelude takes 37 ms, and each evaluation takes 0.5 to 0.9 ms. [M]
- rink: `enum Numeric { Rational(BigRat), Float(f64) }` (R core/src/types/numeric.rs:14-19). Its docs say creating a context costs "30+ ms" (R core/src/lib.rs:26-27). [S]

**5. Lessons for the MVP**
- Value type now:
  ```rust
  enum Num { Exact(BigRational), Approx(f64) }
  struct Quantity { num: Num, unit: Vec<(UnitId, i32)> } // unit empty in MVP
  ```
  This matches rink and fend. It avoids numbat's open migration. [I]
- AST now: `Number`, `Ident`, `Neg`, `Binary(op)`, `Call(ident, args)`, `Convert(expr, target)`. Each node carries a span. [I]
- Resolve `sqrt` through an identifier table, not a special token. Units and currencies can then join the same lookup chain later, as in fend (F/ast.rs:721-744). [S][I]
- Parser: Pratt or precedence climbing. Reserve the lowest slot for `to`/`in`/`->`, as numbat (N/parser.rs:30) and fend (F/parser.rs:371-393) do. [S]
- Accept ×, ÷, − and √. Both engines reject √. [M]
- Keep the root operator after evaluation, so the card can show the "Product" badge. [I]
- Card rule: show the card only if evaluation succeeds, the input contains an operator or function, and the output differs from the input. Errors stay silent. Close open parentheses automatically. [I, modeled on fend]
- Later evaluation signature: `eval(text, &Env{ rates: Option<&RateTable> })`. The rate table is a plain value, like the injected clock. [I]
- Currency is just one more dimension (fend `BASE_CURRENCY`; numbat `dimension Money` in modules/units/currency.nbt:1). [S]
- Put a no-network or preview flag in `Env` from day one. [I]
- With currency, the answer depends on the rates snapshot as well as the text. Key rows by (text, rates_version). [I]
- Defer:
  - implicit multiplication. fend's `parse_apply_cont` is a pile of special cases (F/parser.rs:182-222).
  - prefixes, a typechecker, variables and `ans`, and a VM.
  - interrupts. Without `^`, rational size grows only linearly with input length. [I]

**Documented pain points**
- numbat issue #4, "Use high-precision numeric type", has been open since 2022-09-04. Issue #690 reports the 10^100 case. A prototype using computable reals cost 28% on the prelude benchmark (comment of 2025-06-07). The same thread notes numbat uses NaN as a signal, including for values fetched from APIs, and the prototype's reals have no NaN. [S]
- fend deprecated `ExchangeRateFn` for V2 to add `is_preview` (F/lib.rs:177-238). `set_current_time_v1` is now a no-op (F/lib.rs:388-394). [S]
- fend formats the f64 rate into source text and parses it again (F/units.rs:53-66). Comma decimal separators broke currency conversion. The fix shipped in v1.5.8 (CHANGELOG.md:9, 29). [S]
- rink 0.7.0 changed `Dimensionality` from a BTreeMap alias to a struct. It also moved quantities into a separate namespace from units (R core/BREAKING-CHANGES.md). [S]

I did not look at kalker beyond its manifest: its default build links GMP/MPFR through `rug` (PaddiM8/kalker kalk/Cargo.toml:17-28). [S]

## Prototype: minimal

Both crates build and pass their tests. The spike changed 144 lines across the 4 existing source files and added 184 lines in 2 new files. That is the extension cost.

**Files**
- MVP: `/tmp/calc-proto/minimal/` (`src/{lib,lex,parse,format}.rs`, `tests/fixtures.rs`, `examples/bench.rs`)
- Spike: `/tmp/calc-proto/minimal-spike/` (adds `src/units.rs` and `tests/spike.rs`)
- Diff: `/tmp/calc-proto/spike.diff`. Probe binary: `/tmp/calc-proto/scratch-minimal/`.

**Design** (read-in-source, my code)
There are no dependencies. The pipeline is `evaluate` at `minimal/src/lib.rs:29-52`:
1. Trim the input. Empty input or one ASCII word gives None (calc:48).
2. Tokenize in two steps: a raw lex, then word resolution. `sqrt` and `square root of` come from one phrase table. `x` becomes `×` only after a number or `)` (calc:437-439).
3. If the input ends in a binary operator, drop it and keep the prefix (calc:472-476).
4. Parse by precedence climbing over the whole token list. A `(` or a root after an operand is an implicit `*` at the same binding power (calc:429-436). Leftover tokens give None, so `5 3` and `(2+3` stay silent.
5. Card rule: the expression needs a binary operator or a root. The trailing-operator path is exempt.
6. Evaluate in f64. Any non-finite step, division by zero, or square root of a negative gives None.
7. Round once. Display, copy and words all come from the same rounded digits.

One table owns spelling, binding power, echo and badge (`lex.rs:42-49`):
```rust
OpSpec { op: Op::Mul, symbols: &['*', '×', '·'], words: &["x"], bp: 2, echo: "×", badge: "Product" },
pub enum Expr { Num { echo: String, value: f64 }, Neg(Box<Expr>), Sqrt(Box<Expr>), Bin(Op, Box<Expr>, Box<Expr>) }
pub struct Env {}
pub struct Answer { pub expression: String, pub badge: &'static str, pub display: String, pub copy: String, pub words: Option<String> }
pub fn evaluate(query: &str, env: &Env) -> Option<Answer>
pub fn answer_row_json(a: &Answer) -> String
```
The test prints this row (measured):
`{"index":0,"kind":"calculator","expression":"12 × 34","badge":"Product","display":"408","copy":"408","words":"Four Hundred Eight","actions":[{"chord":"enter","verb":"copy","label":"Copy Answer"}]}`

**Rules I chose (each pinned by a literal test)**
- The badge names the top-level operation: Sum, Difference, Product, Quotient, Square Root, Negation or Number.
- Words are for integers only, in title-case short scale, with hyphenated tens (`Twenty-One`) and `Minus` for negatives. Decimals get none. So do numbers past 15 digits or 40 characters, so `1,234,567` gets none.
- `10 +` shows `10 | Number | Ten`, as in TinyCast (calc:472-473). Only one trailing operator is stripped, so `10 + -` is silent.
- `(2+3`, `10/0`, `sqrt(-1)` and overflow are silent. Unlike fend, the engine does not repair parentheses (calc:474-475).
- The echo is canonical: typographic operators, grouped literals, and parentheses only where precedence needs them.
- Display uses 12 significant digits in plain notation, but never rounds inside the integer part. As written, the fixture rule turned `123456789012345+0` into the copy text `123456789012000` (measured before I fixed it).

**Tests (measured, `cargo test --offline`)**
- MVP: 5 passed, 0 failed. They hold 68 literal assertions: 26 shared fixture rows, 36 rule rows, plus overflow, JSON and purity checks.
- Spike: 9 passed, 0 failed. `tests/fixtures.rs` and `examples/bench.rs` are byte-identical to the MVP's (checked with `cmp`).
- `spike.rs` adds 4 tests with 17 assertions:
  - `5 km to mi` gives `3.10686 mi`.
  - `1 km + 500 m` gives `1.5 km`.
  - `10 usd to eur` gives `9 EUR` with rates {USD 1.0, EUR 0.9}, and no card without a snapshot.
- clippy reports 0 warnings in both crates.

**Spike diff (measured, `diff -ru --exclude=target`, both crates rustfmt-formatted)**
- Existing files: all 4 source files changed, with 124 lines inserted and 20 deleted.
  - `lex.rs`: +9 -0
  - `parse.rs`: +89 -11
  - `lib.rs`: +23 -7
  - `format.rs`: +3 -2
- New files: `src/units.rs` has 98 lines and `tests/spike.rs` has 86.
- Extension cost: 144 changed lines in old files plus 184 new lines. Counting engine code only, it is 144 + 98 = 242.
- The deletions show where the minimal design cost something. 9 of the 20 are in `Expr::eval`, whose `f64` return type became `Q { v, unit: Option<UnitId> }` and gained a rates argument.
  - The lexer and the parser loop only gained branches.
  - Conversion was added at the top of `parse()`, not in the binding-power table.

**Per-keystroke cost (measured, release, 10k evaluations over the 26 fixture inputs, median of 3 runs)**
- MVP: 6,188 µs in total, 619 ns per evaluation.
- Spike: 6,842 µs in total, 684 ns per evaluation.
- The research notes measured fend at 6 to 92 µs and numbat at 0.5 to 0.9 ms. The engines cover very different scopes, so this is not a fair comparison (inferred).

**Lines of code (measured, `wc -l src/*.rs`)**
The MVP has 565 lines: format 156, lex 171, lib 84, parse 154. The spike has 767.

**What I would change**
1. f64 can give wrong answers without any warning. `10000000000000000000000+1-10000000000000000000000` shows `0 | Zero` (measured), but the answer is 1. fend gets it right (per the research notes). I would switch to `Exact(BigRational) | Approx(f64)` before units land. Judging by the spike, the change would touch only `eval` and `format` (inferred).
2. The unit rules conflict. The required `1 km + 500 m -> 1.5 km` makes the left unit win. TinyCast makes the last unit typed win, so `10kg + 500g` is `10,500 g` (calc:192-195). Hongyi needs to pick one. Either way it is one line in `combine` (`minimal-spike/src/parse.rs:225`).
3. `Option<UnitId>` cannot express `km/h` or `m²`, so `1 km * 1 m` is silent. Real units need exponent vectors (calc:212-216). That would rewrite `Q` and `combine` again (inferred).
4. There is no way to return an error. TinyCast shows `Cannot convert Currency to Weight.` (calc:543-544), but `Option<Answer>` cannot carry it. I would change the return to a three-way result (answer, error or no card) now, while it is cheap.
5. Once currency exists, Enter's answer depends on the rates snapshot as well as the text. The row JSON should carry a rates version (inferred).
6. Minor: the echo drops redundant parentheses, so `((2))+3` echoes as `2 + 3`.

## Prototype: pipeline

Both crates build and pass every test. The MVP passes 28 tests and the spike passes 34, with 0 failures. The spike changed 116 lines in pre-existing files and added 135 lines in new files.

**Design**
- **Pipeline.** The pipeline is in `/tmp/calc-proto/pipeline/src/pipeline.rs:37-48`, and every stage is pure. [read-in-source, my code]
```rust
let text = normalize::canonical(query);              // whitespace, glyphs from the tables
if is_search(&text) { return None; }                 // empty or one ASCII word (calc:48-49)
if let Some(v) = PRE_TOKEN_STAGES.iter().find_map(|s| s(&text, env)) { return v.into_answer(); }
let tokens = lexer::tokenize(&text).ok().filter(|t| t.len() <= MAX_TOKENS)?;
let (tokens, trailing) = complete_prefix(tokens);    // `12×34+` (calc:472-479)
TOKEN_STAGES.iter().find_map(|s| s(&tokens, trailing, env)).and_then(Verdict::into_answer)
```
- **Where future features register.**
  - `PRE_TOKEN_STAGES` is empty, with commented slots for dates and time zones (pipeline.rs:23-26).
  - `TOKEN_STAGES` holds `typed_arithmetic`. It has a slot for base conversion before it and one for percent forms after it (pipeline.rs:28-32).
  - `Env {}` is empty (pipeline.rs:10).
- **Tables.** Each kind of name has one table in `src/lexicon.rs`. An operator row owns its spelling, glyphs, word spelling, binding power, echo, badge and arithmetic, as `CalcOperator` does (calc:70). `FUNCTIONS` holds `sqrt`, `√` and `square root of`. `resolve` is the identifier chain.
```rust
OpDef { op: Op::Mul, ascii: '*', glyphs: &['×', '·'], word: Some("x"), bp: 20, echo: "×", badge: "Product", apply: Quantity::try_mul },
```
- **Values.** `enum Num { Exact(BigRational), Approx(f64) }` is rink's shape (`/tmp/calc-proto/rink/numeric.rs:14-19`). `Quantity { num, dim }` uses `enum BaseDim {}`, which is empty in the MVP (value.rs:106). A result counts as overflow past f64 range or past 4096 bits (value.rs:26-50).
- **Parser.** It is a Pratt parser over the table's binding powers. Juxtaposition is `*` at the same binding power, but only before `(` or a function name (calc:429-436).
- **Display.** Values round once to 12 significant digits. Exact integers of up to 15 digits keep every digit. Notation is plain between 1e-9 and 1e15 and scientific outside. Copy text is the same without grouping.
- **Dependencies.** Only num-bigint, num-integer, num-rational and num-traits.

**Rules I chose** [inferred; each has a test in `tests/fixtures.rs`]
- **Words badge.** Title case, short scale and a hyphen in 21 to 99 (`Twenty-Five`). Negatives start with "Minus". Decimals read "Point" and then each digit (`Two Point Five`). There is no badge past 40 characters or in scientific notation, so `sqrt 2` has none.
- **Silence.** `(2+3` is silent, as in TinyCast (calc:475). `10/0` and `sqrt(-1)` are silent.
- **Trailing operator.** `10 +` shows `10` with the badge `Result` (calc:472).
- **Operation badges.** Sum, Difference, Product, Quotient, Square Root, Negation.

**Tests** [measured]
- **MVP: 28 passed, 0 failed.**
  - 26 tests, one per fixture line.
  - One extra-rules test with 16 card cases and 10 silent cases. These include pasted inputs of 100,000 terms and 100,000 nested parentheses.
  - One row JSON test, which prints:
`{"index":0,"kind":"calculator","section":"Calculator","expression":"12 × 34","source_badge":"Product","display":"408","copy":"408","target_badge":"Four Hundred Eight","actions":[{"chord":"Enter","verb":"copy_answer","label":"Copy Answer"}]}`
- **Clippy** reports no warnings on either crate.
- **Spike: 28 original tests plus 6 spike tests passed, 0 failed.**
  - `5 km to mi` gives `3.10686 mi`.
  - `1 km + 500 m` gives `1.5 km`.
  - `10 usd to eur` gives `9 EUR` with the snapshot and no card without it.

**Spike diff** [measured with `diff -ruN -x target`; saved at `/tmp/calc-proto/pipeline-spike.diff`]
- **Pre-existing files:** +83 and -33 lines, in 9 of the 11 src files. value.rs (+30/-9) and present.rs (+19/-8) account for 66 of the 116 lines. lexicon.rs is +12/-10.
- **New files:** `src/units.rs` (+76) and `tests/spike.rs` (+59). `tests/fixtures.rs` and `examples/bench.rs` did not change.
- **What changed:**
  - `BaseDim` gained `Length` and `Money`.
  - `Quantity` gained `unit: Option<DisplayUnit>`.
  - `to` became an operator row at binding power 2.
  - `Ident` gained `Unit`.
  - `Env` gained `rates: Option<&'a Rates>`.
  - `answer()` now returns an `Option`.

**Per-keystroke cost** [measured: `examples/bench.rs`, release build, 10,000 evaluations cycling the 26 fixture inputs]

| Crate | Runs (ms) | Median (ms) | Per evaluation |
|---|---|---|---|
| MVP | 17.09, 17.04, 17.03 | 17.04 | 1.70 µs |
| Spike | 17.81, 17.79, 17.78 | 17.79 | 1.78 µs |

**Lines of code** [measured, `wc -l src/*.rs`, comments included]: MVP 908, spike 1034.

**What I would change**
1. Give `Quantity` the display unit and make `answer()` return an `Option` from day one. Most of the 66 lines in value.rs and present.rs were this work. [measured]
2. Make operator spellings a list. A word operator like `to` forced `ascii: char` to become `Option<char>`, which churned 10 lines in lexicon.rs. [measured]
3. Decide which unit an answer takes. The fixture wants the left unit, but TinyCast lets the last unit win (calc:192-195). The spike follows the fixture.
4. Decide the money format. The fixture wants `9 EUR`, but TinyCast rounds money to 2 decimals (calc:607-609).
5. Add a `Verdict::Error`. With no snapshot, TinyCast's card says so (calc:600-602), and it shows mismatch errors (calc:270-272). The spike stays silent in both cases.
6. Give a number-plus-unit literal its own node. Today `5 km to` echoes `5 km` with the wrong badge, `Product`.
7. Conversion is a grammar row, not a separate stage as calc:55-62 lists. TinyCast's typed stage already takes `to` suffixes (calc:57). I would keep the `TOKEN_STAGES` slot for forms without a number, such as `m to ft`. [inferred]
8. Accept grouping commas such as `1,000 + 234` (calc:455-457) in normalize. They are silent now.
9. Once currency lands, the row JSON must name the snapshot's `fetchedAt`, because the memo keys on it (calc:604-605). [inferred]

Files are in `/tmp/calc-proto/pipeline`:
- `src/pipeline.rs`
- `src/lexicon.rs`
- `src/value.rs`
- `tests/fixtures.rs`

The spike is in `/tmp/calc-proto/pipeline-spike`:
- `src/units.rs`
- `tests/spike.rs`

## Prototype: quantity (base)

I built and tested both crates, and every fixture passes. The MVP passes 61 of 61 tests and the spike passes 74 of 74. The crate has no dependencies (`cargo tree`) [M]. Labels: [M] measured, [S] read in source, [I] inferred.

Paths:
- MVP: /tmp/calc-proto/quantity
- Spike: /tmp/calc-proto/quantity-spike
- Spike diff: /tmp/calc-proto/quantity-spike.diff
- Control with every slot removed: /tmp/calc-proto/quantity-scalar

**Design** [S]

Pipeline (src/lib.rs:53): trim, cap at 256 bytes, exit early on a single ASCII word, tokenize, strip one trailing infix symbol, Pratt parse, check for a plain number, evaluate, round once, then render display, copy and words.

```rust
pub fn evaluate(text: &str, env: &Env) -> Option<Answer>
pub struct Env<'a> { pub rates: Option<&'a Rates> }  // None in MVP
pub enum Num { Exact(Ratio /* i128 */), Approx(f64) }
pub const BASES: usize = 0;
pub struct Dim(pub [i8; BASES]);                     // zero-sized in MVP
pub struct Quantity { num: Num, dim: Dim, unit: Option<Unit> }
```
(src/quantity.rs:8, 119, 123, 154)

One operator table owns spelling, binding power, echo, badge and meaning (src/ops.rs:12):
```rust
pub static MUL: Operator = Operator { spellings: &["*", "×", "·", "x"], bp: 20,
    echo: "×", badge: "Product", apply: Apply::Infix(Quantity::checked_mul) };
```
- The tokenizer takes its symbols from this table, and words stay words.
- Where an operand is expected, the parser first tries prefix spellings (`sqrt`, `square root of`). Then it calls `catalog::lookup(word, env)`, which returns None in the MVP (src/catalog.rs:11).
- Juxtaposition means `MUL` when the next token is a paren, a root or a catalog name. A number never starts a product (src/parse.rs:97).

Rules I chose, each pinned in tests/fixtures.rs:
- **Words badge:** integers only, after rounding. Title-case short scale, "Minus" for negatives, hyphenated tens ("Twenty-Five"). No words past 64 characters or in scientific notation.
- **Display:** 12 significant digits, but never fewer than the integer digits. Plain notation for exponents -7 to 20.
- **Silence:** division by zero, `sqrt(-1)` and an open paren show no card.
- **Trailing operator:** only a symbol counts, so `12 x` and `0x` stay searches.

Row JSON, printed by test `row_json_for_query` [M]:
`{"kind":"answer","expression":"12 × 34","badge":"Product","display":"408","copy":"408","words":"Four Hundred Eight","actions":[{"chord":"enter","verb":"copy","label":"Copy Answer"}]}`

**Tests** [M]
- MVP: 61 passed, 0 failed. That is 26 shared fixtures, 34 lyncast rules and 1 JSON test. Clippy is clean.
- Spike: 74 passed, 0 failed. That is the 61 MVP tests, unchanged, plus 13 spike tests.
- I generated 300,000 random inputs and ran them in a debug build. None panicked.

**Spike diff** [M] (`diff -ru -x target`)

Pre-existing files changed by +24 / -5 lines across 6 files:

| File | Lines | What changed |
|---|---|---|
| catalog.rs | +3 / -3 | the body calls `units::lookup` |
| ops.rs | +11 / -1 | the CONVERT row (`to`, `in`, `->`, `→`, bp 5) |
| quantity.rs | +3 / -1 | `BASES` goes from 0 to 2, plus `Precision::Decimals` |
| format.rs | +2 | rounding for Decimals |
| parse.rs | +4 | `5 km` alone stays silent |
| lib.rs | +1 | `mod units;` |

New files: src/units.rs (38 lines) and tests/spike.rs (48 lines). No line of the tokenizer or the Pratt loop changed.

**Do the slots pay?** [M]
- **Size:** the control crate (Quantity replaced by Num, no catalog, no rates) has 756 lines in src/. The MVP has 901. The slots cost 145 lines, 109 of them code.
- **Extension cost:** starting from the control, the same spike changes +209 / -58 lines in pre-existing files, against +24 / -5 from the MVP. Part of that 209 is rustfmt reflow.
- **Time:** the control is no faster (table below).

**Cost per keystroke** [M]

Release build, 10,000 evaluations cycling the 26 shared inputs, median of 3 runs:

| Crate | 10,000 evaluations | Per evaluation |
|---|---|---|
| MVP | 11.25 ms | 1.13 µs |
| Control (no slots) | 11.36 ms | 1.14 µs |
| Spike | 13.48 ms | 1.35 µs |

**Lines of code** [M] `wc -l src/*.rs` gives 901: parse 220, format 216, quantity 201, lib 103, ops 91, lex 57, catalog 13.

**Conflicts with TinyCast** (decisions for Hongyi)
- The spike spec asks for `1 km + 500 m` → `1.5 km`, so the left operand's unit wins, as in fend. TinyCast keeps the last unit typed and would show `1,500 m` (calc:192-195) [S]. The rule is the single expression `self.unit.or(o.unit)` at src/quantity.rs:170.
- `9 EUR` drops trailing zeros. TinyCast pads money: `$10 +` shows `10.00 USD` (calc:476) [S].

**What I would change**
1. Return `Result` with a mismatch error instead of `Option`. The error card for `1 kg + 1 m` (calc:270-272) is the one slot I did not build, and adding it later changes every `Apply` signature [I].
2. Share units as `Arc<UnitDef>`, as TinyCast does (calc:80). Today every unit name is a `String` that gets cloned [S][I].
3. Keep rates as exact decimals. They are f64 now, so currency answers rely on rounding rather than exact arithmetic [S].
4. Put the `5 km` silence rule in the MVP's `is_plain_number`, so a units extension never touches parse.rs [I].
5. Index spellings once. `op_at` scans every spelling at each token position. The spike is 0.22 µs slower per evaluation [M], and the four new spellings are the likely cause [I].
6. Name the units in conversion badges, such as `Kilometres → Miles`. The spike only shows `Conversion` [I].
7. Keep the two guards: the 256-byte cap and a parse depth limit of 32. Without the cap, 10,000 nested parens overflowed the stack. Without the depth limit, 126 nested parens overflowed a 256 KB thread in release [M].

## Judge: verification, scorecard, synthesis

```json
{
  "verification_md": "All runs on this host, 2026-10-14. Each crate used its own `CARGO_TARGET_DIR=<crate>/target` with `--offline`. Logs are in /tmp/calc-proto/judge-logs/. My probe crates are /tmp/calc-proto/judge-probe and /tmp/calc-proto/judge-probe-spike. I wrote nothing outside /tmp/calc-proto.\n\n**cargo test --release** [M]\n| Crate | Passed | Failed |\n|---|---|---|\n| quantity | 61 | 0 |\n| quantity-spike | 74 (61 + 13) | 0 |\n| quantity-scalar (control) | 61 | 0 |\n| minimal | 5 | 0 |\n| minimal-spike | 9 (5 + 4) | 0 |\n| pipeline | 28 | 0 |\n| pipeline-spike | 34 (28 + 6) | 0 |\n\nMinimal's 5 tests are table-driven (many assertions each), so the counts are not comparable. `cargo clippy --release --all-targets` reported 0 warnings on all six MVP and spike crates. In all three pairs, `tests/fixtures.rs` is byte-identical between MVP and spike (`cmp`).\n\n**Diff stats** (`diff -ruN -x target -x Cargo.lock`, changed lines in files that existed before the spike) [M]\n- quantity to spike: +24 / -5 in 6 files (catalog +3/-3, format +2/-0, lib +1/-0, ops +11/-1, parse +4/-0, quantity +3/-1). New: units.rs 38, tests/spike.rs 48.\n- minimal to spike: +124 / -20 in all 4 files (format +3/-2, lex +9/-0, lib +23/-7, parse +89/-11). New: units.rs 98, tests/spike.rs 86.\n- pipeline to spike: +83 / -33 in 9 of 11 files (value +30/-9, present +19/-8, lexicon +12/-10, parser +7/-1, pipeline +6/-2, lexer +3/-2, eval +2/-1, format +2/-0, lib +2/-0). New: units.rs 76, tests/spike.rs 59.\n- quantity-scalar to quantity (the cost of the slots): +202 / -57, net +145.\n- All three reports' diff numbers match mine.\n\n**Lines** (`wc -l src/*.rs`) [M]: quantity 901, quantity-spike 958, quantity-scalar 756, minimal 565, minimal-spike 767, pipeline 908, pipeline-spike 1034. Dependencies: quantity and minimal have none. Pipeline uses num-bigint, num-integer, num-rational and num-traits.\n\n**Per-keystroke bench** (each crate's examples/bench.rs, release, 10,000 evaluations over the 26 fixtures, 3 runs) [M]\n| Crate | Runs | Per evaluation |\n|---|---|---|\n| quantity | 11.256, 11.326, 11.276 ms | 1.13 µs |\n| quantity-scalar | 11.359, 11.284, 11.394 ms | 1.13 to 1.14 µs |\n| quantity-spike | 13.330, 13.422, 13.396 ms | 1.34 µs |\n| minimal | 6.229, 6.235, 6.212 ms | 622 ns |\n| minimal-spike | 6.472, 6.475, 6.378 ms | 647 ns |\n| pipeline | 16.87, 17.02, 16.87 ms | 1.69 µs |\n| pipeline-spike | 17.82, 17.77, 17.97 ms | 1.78 µs |\n\n**Pathological inputs** (in-process, median of 5) [M]\n| Input | quantity | minimal | pipeline |\n|---|---|---|---|\n| 100,000 digits + `+1` | 0 µs | 109 µs | 202,618 µs |\n| 10,000 digits | 0 µs | 11 µs | 2,700 µs |\n| `1+` x 50,000 | 0 µs | 8,229 µs (shows a card) | 8,107 µs |\n| 100,000 nested parens | 0 µs | process abort: `fatal runtime error: stack overflow` | 2,805 µs |\n| 126 nested parens, 256 KB thread | silent (depth cap) | card | card |\n| 127 nested parens, 256 KB thread | silent | card | silent (token cap) |\n\nFuzz: 200,000 random inputs from a 26-token alphabet, release build with overflow checks on: 0 panics in each engine [M].\n\n**Shared fixtures through one probe binary** [M]: all 26 lines match the brief in all three. The only difference is designer-chosen words. Pipeline gives `Two Point Five` and `Zero Point Three`. Quantity and minimal give none.\n\n**Divergent probes** [M]\n- `1000000000000000000000000000000000000000+1-1000000000000000000000000000000000000000`: Q `0`, M `0`, P `1`.\n- `10000000000000000000000+1-10000000000000000000000`: Q `1`, M `0`, P `1`.\n- `999999999999999*1000`: Q `999,999,999,999,999,000`, M `999,999,999,999,998,980` (wrong digits), P `1e+18`.\n- `1234567890123456+0`: Q and M show it exactly. P shows and copies `1.23456789012e+15`.\n- `0x`: Q silent. M `0 | Number | 0 | 0 | Zero`. P `0 | Result | 0 | 0 | Zero`.\n- `12 x` and `10 x`: Q silent. M and P show the number.\n- `--5`: Q `−(−5) | Negation | 5`. M and P silent.\n- `.5+.5`: Q echoes `.5 + .5`. M and P echo `0.5 + 0.5`.\n- `((2))+3`: Q and M echo `2 + 3`. P echoes `((2)) + 3`.\n- `4(2+3)`: Q and P echo `4(2 + 3)`. M echoes `4 × (2 + 3)`.\n\n**Spike probes** (rates USD 1.0, EUR 0.9) [M]\n- All three give `5 km to mi` = `3.10686 mi`, `1 km + 500 m` = `1.5 km`, and `10 usd to eur` = `9 EUR` with the snapshot and silence without it.\n- `5 km / 1 km`: Q and P silent. M gives `5`.\n- `1 m + 2`: Q and P silent. M gives `3 m`.\n- `3 km x 2`: Q and P give `6 km`. M is silent.\n- `10 m in km`: Q and M convert. P is silent.\n- `3 mi -> km`: only Q converts.\n- `5 km to` (trailing): P shows `5 km | Product`. Q and M are silent.\n- `1 usd + 1 eur`: Q `2.11 USD`. M and P `2.11111 USD`.\n\n**Purity**: grepping all six src trees for std::fs/net/time/env/process/io, OnceLock, thread_local, static mut, Mutex, RefCell, Cell and unsafe finds nothing [M].\n\n**Not re-run**: quantity's \"control to spike +209/-58\" (no control-spike directory exists) and quantity's 300,000-input debug fuzz (I ran my own 200,000-input fuzz instead).",
  "scorecard_md": "Scores run 1 to 5, where 5 is best. Q = quantity, M = minimal, P = pipeline.\n\n| Criterion | quantity | minimal | pipeline |\n|---|---|---|---|\n| Fixture correctness [M] | 4 | 3 | 4 |\n| Extension cost [M] | 5 | 2 | 3 |\n| MVP simplicity | 3 | 5 | 2 |\n| Purity and injected env | 5 | 3 | 4 |\n| Root-query integration | 4 | 3 | 4 |\n| Per-keystroke cost [M] | 5 | 4 | 3 |\n| Readability | 3 | 4 | 4 |\n| **Total** | **29** | **24** | **24** |\n\n**Fixture correctness.** All three pass 26 of 26 [M].\n- Q: `0x`, `12 x` and `10 + -` behave as calc:437-439 and calc:472-476 require [M]. The i128 `Ratio` falls back to f64 without a signal (quantity/src/quantity.rs:79-86), so `10^39+1-10^39`, typed in digits, shows `0` [M]. `--5` earns a card. `.5+.5` echoes `.5 + .5` [M].\n- M: f64 gives `0` for `10^22+1-10^22`. It shows invented digits as if exact (`999,999,999,999,998,980`). `0x` earns a card, against calc:438-439. 100,000 nested parens abort the process [M]. Minimal has no length cap and no depth limit (minimal/src/lib.rs:29-34, minimal/src/parse.rs:35-48) [S].\n- P: exact on every big-number probe [M]. But `0x` earns a card, and exact integers of 16 or more digits copy in scientific notation (`1e+18`) [M].\n\n**Extension cost** (changed lines in old files during the spike) [M].\n- Q: +24/-5 in 6 files. The tokenizer and the Pratt loop do not change. The slots were paid up front: +202/-57 from the scalar control.\n- P: +83/-33 in 9 of 11 files. `ascii: char` became `Option<char>` (lexicon.rs), and `Quantity` gained `unit`.\n- M: +124/-20 in all 4 files. `eval` changed its return type, and parse.rs gained +89/-11.\n\n**MVP simplicity.**\n- M: 565 lines in 4 files, no dependencies, f64 only [M].\n- Q: 901 lines in 7 files, no dependencies. It carries `Unit`, `Precision`, `Rates` and `catalog` before anything uses them [S].\n- P: 908 lines in 11 files and 4 num crates. Its stage arrays hold 0 and 1 entries (pipeline/src/pipeline.rs:23-32) [S].\n\n**Purity and injected env.** No IO, clock, globals or unsafe in any crate [M].\n- Q: has `Env<'a> { rates: Option<&Rates> }` with `fetched_at` from day one (quantity/src/lib.rs:20-32) [S].\n- P: `Env {}` is empty, but its typed `EvalError` (including `Mismatch`) is ready for error cards (pipeline/src/value.rs:10-16) [S].\n- M: `Env {}` is empty. Its spike switched to an owned `Option<Rates>` that is cloned per call [S].\n\n**Root-query integration.**\n- Q: only a trailing symbol counts as an operator (quantity/src/parse.rs:35-41), copy is ungrouped, and words are integer-only. Its row JSON has no `index` or `section` [S].\n- P: its JSON has `index`, `section` and `source_badge`/`target_badge`, which match TinyCast's `CalcResult` (calc:613-614). It loses points on `0x` and the lossy integer copy [M].\n- M: `0x` earns a card. The badge is `Number`, where TinyCast's generic badge is `Result` (calc:461-463). It crashes on large pastes [M].\n\n**Per-keystroke cost.**\n- Q: 1.13 µs typical. The 256-byte cap makes the worst case 0 µs [M].\n- M: 622 ns typical, but 8.2 ms on a 50,000-term paste [M].\n- P: 1.69 µs typical, but 203 ms on a 100,000-digit paste, because it parses BigInts before its token cap [M].\n\n**Readability.**\n- Q: one table drives the tokenizer, parser, echo and badge. But the code is dense: `Expr::Binary(op, bool, l, r)` takes a positional bool, `op_at` rescans every spelling at each token (quantity/src/parse.rs:111-131), and lines run to 120 columns [S].\n- M: the smallest code. But sqrt and negation sit outside the operator table (minimal/src/lex.rs:66-68, minimal/src/parse.rs:45-46) [S].\n- P: the lib.rs header is a stage diagram that maps to TinyCast's pipeline (pipeline/src/lib.rs:3-11). Fields are named and errors are typed [S].",
  "base": "quantity (/tmp/calc-proto/quantity)",
  "grafts": [
    "pipeline: Num::Exact(BigRational) with the 4096-bit overflow cap (pipeline/src/value.rs:20-50), replacing quantity's i128 Ratio and its silent f64 fallback (quantity/src/quantity.rs:79-86); fixes 10^39+1-10^39 showing 0 [M]",
    "pipeline: typed EvalError plus a three-way outcome (pipeline/src/value.rs:10-16, pipeline/src/pipeline.rs:13-16), so Apply returns Result now and error cards (calc:270-272, 600-602) need no signature churn later",
    "pipeline: row JSON with index 0, section, source_badge and target_badge (pipeline/src/present.rs:72-86), matching TinyCast CalcResult naming (calc:613-614)",
    "pipeline: recursive bare-number check (pipeline/src/parser.rs:40-46) so --5 stays silent [M]",
    "minimal: canonical literal echo (minimal/src/lex.rs:158-170) so .5 echoes 0.5 and 007 echoes 7 [M]",
    "minimal-spike: unit names bind to the preceding operand tighter than * (Expr::Measure, minimal-spike/src/parse.rs:12, 67-70), so 5 km / 1 km is 5 and 1 m + 2 is 3 m as in calc:200, 204, 436 [M]",
    "pipeline-spike: &'static UnitDef rows with a plural name for the target badge (pipeline-spike/src/units.rs:20-30), replacing quantity's cloned String unit names"
  ],
  "design_md": "## Base\nBase: quantity (`/tmp/calc-proto/quantity`). It is the only prototype that keeps `0x` and `12 x` silent [M], as calc:437-439 requires. It caps input before doing any work, so a 100,000-digit paste costs 0 µs [M] (quantity/src/lib.rs:57). It took the units and currency spike with +24/-5 lines in old files [M].\n\n## Grafts\n1. pipeline `Num::Exact(BigRational)` with the 4096-bit cap (pipeline/src/value.rs:20-50). Quantity's i128 `Ratio` falls back to f64 without a signal (quantity/src/quantity.rs:79-86). So `10^39+1-10^39`, typed in digits, shows `0` [M]. Pipeline shows `1` [M].\n2. pipeline typed `EvalError` and a three-way verdict (pipeline/src/value.rs:10-16, pipeline/src/pipeline.rs:13-16). `Apply` returns `Result` from day one. Quantity's report says this change gets costly later [S].\n3. pipeline row JSON fields `index`, `section`, `source_badge`, `target_badge` (pipeline/src/present.rs:72-86). They match TinyCast's `sourceBadge`/`targetBadge` (calc:613-614).\n4. pipeline recursive bare-number check (pipeline/src/parser.rs:40-46). Quantity shows a card for `--5` [M].\n5. minimal canonical literal echo (minimal/src/lex.rs:158-170). Quantity echoes `.5 + .5` [M].\n6. minimal-spike unit binding (`Expr::Measure`, minimal-spike/src/parse.rs:12, 67-70). A unit name binds to the operand before it, tighter than `*`. Quantity and pipeline treat it as juxtaposition at `*` strength, so `5 km / 1 km` stays silent [M]. TinyCast keeps unit names in \"their own operand positions\" (calc:436) and binds composites above `*` (calc:200).\n7. pipeline-spike `&'static UnitDef` rows with a plural name for the target badge (pipeline-spike/src/units.rs:20-30). They replace quantity's cloned `String` names.\n\n## Types\n```rust\npub fn evaluate(text: &str, env: &Env) -> Outcome;\npub enum Outcome { Card(Answer), Error(String), Silent }\npub struct Env<'a> { pub rates: Option<&'a Rates> }  // None in MVP\npub struct Rates { pub fetched_at: i64, pub per_base: BTreeMap<Code, BigRational> }\npub enum Num { Exact(BigRational), Approx(f64) }\npub struct Dim(pub [i8; BASES]);                     // BASES = 0 in MVP\npub struct Quantity { num: Num, dim: Dim, unit: Option<&'static UnitDef> }\npub enum EvalError { DivideByZero, Domain, Overflow, Mismatch, NoRate(Code), NoSnapshot }\npub struct Answer { expression: String, source_badge: String, display: String,\n                    copy: String, target_badge: Option<String> }\n```\nOne `Operator` table owns spellings, binding power, echo, badge and `apply` (quantity/src/ops.rs:12-22). Index the spellings once, not per token (quantity/src/parse.rs:111-117) [I].\n\n## Pipeline\n1. Trim. Over 256 bytes is Silent. Empty input or one ASCII word is Silent (calc:48).\n2. Tokenize once: numbers, words, parens, and symbols from the table.\n3. Strip one trailing infix symbol (calc:472-476). Words never count, so `12 x` and `0x` stay searches.\n4. Pratt parse with a depth cap of 32. An operand is a prefix row (`-`, `√`, `sqrt`, `square root of`), a number, a closed group or a catalog name. Juxtaposition is `*` before `(` or a root. Numbers never juxtapose (calc:435-436).\n5. A bare number, negated any number of times, is Silent unless a trailing operator was stripped.\n6. Evaluate to `Result<Quantity, EvalError>`.\n7. Round once by the unit's precision. Display is grouped. Copy is the same digits ungrouped. Words come from the same digits (calc:615).\n8. Ok is a Card. In the MVP every error is Silent.\n\n## MVP rules (each a CLI fixture)\n- Source badge: the root operator's name (Sum, Difference, Product, Quotient, Square Root, Negation). A stripped-prefix lone number reads `Result` (calc:461-463).\n- Target badge: integers only, after rounding. Title-case short scale, hyphenated tens, `Minus` for negatives. None for fractions, scientific notation, or text over 40 characters.\n- Display: 12 significant digits, never fewer than the integer digits. Plain notation for exponents -7 to 20.\n- Silent: `10/0` (a prefix of `10/0.5`), `sqrt(-1)`, open parens, overflow, unknown words.\n\n## Root-query integration\nRoot rows are the optional card at flat index 0, then ranked apps shifted by one (pal:13-16). The chord table maps (Calculator, Enter) to `CopyAnswer`. Ctrl+Enter stays unmapped until chaining. The daemon writes `answer.copy` to the clipboard.\n\n`lyncast query '12×34' --json` prints one object per row:\n`{\"index\":0,\"kind\":\"calculator\",\"section\":\"Calculator\",\"expression\":\"12 × 34\",\"source_badge\":\"Product\",\"display\":\"408\",\"copy\":\"408\",\"target_badge\":\"Four Hundred Eight\",\"actions\":[{\"chord\":\"enter\",\"verb\":\"copy_answer\",\"label\":\"Copy Answer\"}]}`\n`lyncast activate --query '12×34' --index 0` re-evaluates the text and copies `copy`. No daemon state is read, because `Env` is empty in the MVP.\n\n## Where later features register\n- Units: rows in `units.rs`. `catalog::lookup` asks units first. `BASES` grows. One CONVERT row in `ops.rs` at bp 5 (`to`, `->`, `→`). The spike needed +24/-5 old lines [M].\n- Currency: the same catalog, after units (calc:535-544), priced from `Env.rates`. Rates parse from decimal text into `BigRational`, not f64. `Precision::Money` uses 2 decimals, widening to 4 significant digits below a cent (calc:607-609). `NoSnapshot` and `NoRate` become error cards (calc:600-602). `Mismatch` becomes `Cannot convert X to Y.` (calc:270-272, 544).\n- CLI with currency: rows gain `\"rates_fetched_at\"`. Activation passes `--rates-at <ts>`. If the daemon's snapshot differs, activation fails with exit code 2 and copies nothing. Tests inject `--rates tests/rates.json`.\n- Dates and time zones: a pre-token stage ahead of tokenizing, with an injected `Env.now` (calc:51-55). The single-word gate gains its five exceptions (calc:48-49).\n- `%`, `mod`, `^`: new operator rows (calc:467-470).\n\n## Fixtures to add\n- `1000000000000000000000000000000000000000+1-1000000000000000000000000000000000000000` gives `... | Difference | 1 | 1 | One`.\n- `--5` gives ∅. `.5+.5` gives `0.5 + 0.5 | Sum | 1 | 1 | One`. `0x` and `12 x` give ∅.\n- 100,000 nested parens and a 100,000-digit paste give ∅ in under 1 ms.\n- The JSON row above, byte for byte.\n- For units later: `5 km / 1 km` gives `5`. `1 m + 2` gives `3 m` (calc:204).\n\n## Open questions for Hongyi\n1. Which unit does a sum take: the last one typed (TinyCast, calc:192-195) or the left one (spike fixture)? It is one expression (quantity/src/quantity.rs:170). I recommend TinyCast.\n2. Money as `9.00 EUR` (calc:607-609) or `9 EUR`? I recommend 2 decimals.\n3. Words cap at 40 or 64 characters? Words for decimals (`Two Point Five`, pipeline) or none?\n4. `in` as a conversion keyword collides with inches.\n5. Should division by zero stay silent or show an error card? I recommend silent.\n6. Should currency activation pin the snapshot as above, or copy the current snapshot's answer?"
}
```

## Review: integration and extensibility

```json
{
  "verdict": "Not ready as written. The MVP core holds up. The engine is pure, `Env` is empty, and the card at index 0 needs nothing from the ranker except `Outcome` [S] (quantity/src/lib.rs:53-74). Re-running `12×34` with an empty Env gives the same answer every time, so MVP activation is sound. Four integration claims break once later features or the delivery seam arrive. (1) Currency is priced at parse time, so the NoSnapshot and NoRate error cards can never appear [M]. (2) The engine hard-codes the row JSON's index, section and verbs. That breaks the seam's rule that verbs stay in the daemon [S]. (3) `--query/--index` activation clashes with the agreed `calc:<expr>` key [S]. (4) Only rates are pinned. Clock, time zone, region and number format are not, so date answers will copy something different from what the card showed [I]. Units file count: units would edit all 7 existing src files (lib, lex, parse, ops, quantity, catalog, format) plus a new units.rs [I]. The measured spike edited 6 of 7 for a smaller feature, and it fails the design's own unit fixtures [M]. Probes I wrote: /tmp/calc-proto/review-probe/src/main.rs and /tmp/calc-proto/review-pow/src/main.rs.",
  "issues": [
    {
      "title": "Currency is priced at parse time, so NoSnapshot and NoRate error cards cannot happen and the tree shape depends on rates",
      "severity": "major",
      "scenario": "Fresh boot, before the first fetch lands. Hongyi types `10 usd to eur`. The design promises a NoSnapshot error card (calc:600-602). Instead the query stays silent. Later, with a USD/EUR snapshot, `10 usd to gbp` should say `No exchange rate for GBP.` It also stays silent.",
      "evidence": "Measured on quantity-spike through /tmp/calc-proto/review-probe: `10 usd to eur` with no rates gives ∅. `10 usd to gbp` with a USD/EUR snapshot gives ∅ [M]. The spike test pins this too (quantity-spike/tests/spike.rs:38-39) [S]. Cause, read in source: the base resolves words during parsing via `catalog::lookup(w, self.env)` and stores an Atom that already has its value. Parse returns `Option`, so a typed error has no way out (quantity/src/parse.rs:13-17, 86-88). The spike treats a code as existing only while the snapshot quotes it (quantity-spike/src/units.rs:17-20). The choice of implicit product also reads the env (quantity/src/parse.rs:103), so whether `2 usd` parses depends on rates. The Types section says `unit: Option<&'static UnitDef>`. That drops the resolved factor pipeline-spike keeps in `DisplayUnit { def, factor }` (pipeline-spike/src/units.rs:52-57). Meanwhile `format::show` divides by `u.per` without access to the env (quantity/src/format.rs:17-20). So nothing can render EUR from a `Factor::Rate(\"EUR\")` row [S].",
      "fix": "Make catalog lookup independent of the env: `catalog::lookup(word) -> Option<&'static UnitDef>`, backed by a static currency table generated as in calc:487-500. Price atoms in `eval(&Expr, &Env) -> Result<Quantity, EvalError>`, as pipeline-spike's `units::one` does (pipeline-spike/src/units.rs:66-76). Return NoSnapshot when `env.rates` is None and NoRate(code) when the code is unquoted. Carry `{ def: &'static UnitDef, factor: Num }` in Quantity so the formatter never needs the env. Add CLI fixtures for both error cards."
    },
    {
      "title": "The engine builds the row JSON with a fixed index, section and verbs, duplicating the chord table and breaking the delivery seam",
      "severity": "major",
      "scenario": "Paste v1 adds Verb::Paste to the chord table, or chaining maps Ctrl+Enter. `lyncast query '12×34' --json` still lists only `copy_answer`, because that list is a string literal inside the engine. The CLI fixtures keep passing while the palette offers a verb the CLI never shows. Shipping paste then needs an edit to the engine, which the seam promised to avoid. Separately, when currency adds error cards, `Outcome::Error` has no row kind, no JSON shape and no Enter verb, so there is nothing to write a fixture against.",
      "evidence": "Graft 3 copies pipeline's `root_row_json`. It hard-codes `\"index\":0`, `\"section\":\"Calculator\"` and the actions array (pipeline/src/present.rs:72-86). The base does the same (quantity/src/lib.rs:78-88) [S]. The lead-judged delivery seam says: \"Row actions stay in the daemon, so paste verbs never reach core\" and \"Paste lands later without edits to lyncast-core\" (/tmp/paste-seam-design.md:7, 39) [S].",
      "fix": "The engine returns only `Outcome`. The daemon's root-row builder assigns `index` and `section` and fills `actions` from the chord table, so the CLI and the palette read one source. Define a row kind for error cards and say what Enter does on one (for example, no verb), then pin that in a JSON fixture."
    },
    {
      "title": "Activation by --query/--index conflicts with the agreed key-based activation, and the index form depends on ranker state between calls",
      "severity": "major",
      "scenario": "The delivery seam's test runs `lyncast activate 'calc:2+3*4'` and expects `copied 14`. This design runs `lyncast activate --query '12×34' --index 0`. That gives two CLI contracts and two request shapes for one action. With the index form, an app row's position can shift between `query` and `activate`, for example after a frecency bump or an app install. A script then launches a different app than the one it saw. If a key carried the echo instead of the raw text, `10 +` would echo `10`, and `10` re-evaluates to Silent.",
      "evidence": "The seam defines keys `app:<id>`, `clip:<id>` and `calc:<expr>`, plus `Request::Activate { key, action }`, `Core::resolve(&Key)` and `Delivery::Answer(CalcAnswer)` (/tmp/paste-seam-design.md:21-37) [S]. A bare `12` is ∅ while `10 +` gives a `10 | Result` card (quantity/tests/fixtures.rs:54, 67) [S]. That the ranking drifts is inferred [I].",
      "fix": "Emit `\"key\":\"calc:<raw query>\"` in the calculator row and activate through the seam's `Request::Activate { key, action }`. If `--index` is kept, the CLI process should resolve it to a key from the rows it just printed, so it never reaches the daemon protocol. The key must carry the raw text, not the echo."
    },
    {
      "title": "Only rates are pinned; clock, zone, region and number format change between query and activation",
      "severity": "major",
      "scenario": "Once dates land, Hongyi types `time` at 10:41:59.8 and presses Enter at 10:42:00.1. The card shows 10:41. The daemon re-evaluates with a fresh `Env.now` and copies 10:42. `hrs till 9am` and `time in tokyo` drift the same way. A wall-clock time that falls in a DST gap stays silent (calc:149), so re-evaluating across that boundary can turn a Card into Silent, and index 0 becomes an app. Tests that assert literal date output have no `--now` or `--tz` flag. The `--rates tests/rates.json` flag has no stated owner. If it is a per-call flag while activation compares against the daemon's own snapshot, the activation fixture exits 2. If every row carries `rates_fetched_at`, activating plain `12×34` after a refresh exits 2 even though that answer read no rates. If the first query lands before any snapshot, the NoSnapshot card stays on screen after rates arrive, because nothing triggers a re-evaluation.",
      "evidence": "The design pins only `--rates-at` and lists only `rates` and, later, `now` in Env. TinyCast injects the clock, the calendar with its home zone and first weekday, the region currency and the number format (calc:11-15, 28-29, 171-174, 555-558, 625-632) [S]. TinyCast re-keys its memo when `fetchedAt` changes (calc:604-605) [S]. The drift scenarios are inferred [I].",
      "fix": "Pin the whole Env, not one field: put an opaque `env` token in each row (rates `fetched_at`, now, zone, region, format). The daemon re-evaluates under the pinned now and zone, which are injectable. It rejects only when the pinned rates snapshot is gone. Have evaluation report which Env fields it read, so scalar rows carry no rates pin. Make `--rates`, `--now` and `--tz` daemon start flags so tests are hermetic. Push a 'rates changed' event so the palette re-queries."
    },
    {
      "title": "The 4096-bit cap is checked after the arithmetic, so a future `^` row is not bounded",
      "severity": "minor",
      "scenario": "After `^` is added as \"a new operator row\", typing `9^9^7` (5 bytes) computes 9^4782969 on every keystroke before the cap can reject it. `9^9^9` needs about 1.2e9 bits, roughly 150 MB of work per keystroke.",
      "evidence": "`Num::checked` runs only after `exact(x, y)` returns (pipeline/src/value.rs:44-56) [S]. num-bigint 0.4.8 computed 9^4782969 (15,161,654 bits) in 612 ms, and 9^531441 in 20.6 ms [M] (/tmp/calc-proto/review-pow). The `9^9^9` size is extrapolated [I].",
      "fix": "The pow row checks `bits(base) * exponent <= 4096` before computing. It also requires an integer exponent below a fixed bound. Add a fixture: `9^9^9` gives ∅ in under 1 ms."
    },
    {
      "title": "The +24/-5 units cost was measured on a spike that is not this design; units would touch all 7 existing src files",
      "severity": "minor",
      "scenario": "An implementer budgets units at about 30 changed lines, then finds the design's own fixtures need parser, lexer and formatter work. `5 km / 1 km` should give 5 and gives ∅. `1 m + 2` should give `3 m` and gives ∅. `km/h`, `m²` and `fl oz` need lookahead in the tokenizer that consults the unit table. Composed results such as `0.5 kg/m³` or `25 USD/hr` cannot be named with `Option<&'static UnitDef>`, so Quantity's unit type changes again.",
      "evidence": "I reproduced the diffstat: catalog +3-3, format +2, lib +1, ops +11-1, parse +4, quantity +3-1, which touches 6 of 7 old src files [M]. That spike treats units as implicit products, which graft 6 rejects. Through review-probe, `5 km / 1 km` gives ∅ and `1 m + 2` gives ∅ [M]. TinyCast's compound spellings and composed units: calc:221-223, 231, 284-289, 695 [S]. The file count is inferred [I].",
      "fix": "Re-run the units spike on the synthesized base with all grafts applied and the listed unit fixtures, and quote that diff. Pick a unit representation that composes, such as a product of (UnitDef, exponent) terms, before the type spreads through `Apply` and the formatter."
    },
    {
      "title": "A fixed badge string per operator cannot produce TinyCast's conversion, date or zone badges",
      "severity": "minor",
      "scenario": "`10 usd to eur` shows the badge `Conversion`. TinyCast badges a currency answer like `US Dollar → Bangladeshi Taka`, a date with its weekday and a zone answer with its city. Each of those later features would need the badge mechanism changed.",
      "evidence": "`Operator.badge: &'static str` (quantity/src/ops.rs:19-20) [S]. quantity-spike gives `10 USD to EUR | Conversion` [M]. TinyCast's badges: calc:104-108, 132, 380, 555-556 [S].",
      "fix": "Compute the badge from the root expression and its value (`fn(&Expr, &Quantity) -> String`), and keep the fixed names as the default for arithmetic."
    },
    {
      "title": "A (row, chord) table cannot express per-answer chaining",
      "severity": "minor",
      "scenario": "When chaining lands, mapping (Calculator, Ctrl+Enter) to Chain offers chaining on `time in tokyo` and on boolean answers. TinyCast withholds it there because a clock typed back into the query reads as local time.",
      "evidence": "TinyCast clears `canChain` on every date, time-zone and boolean answer (calc:620-623) [S]. The design's table keys only on row kind and chord, and `Answer` has no capability field [S].",
      "fix": "Add an answer kind or `can_chain` flag to `Answer`, and key the table on (row kind, answer kind, chord). Pin it with a JSON fixture once dates exist."
    }
  ]
}
```

## Review: adversarial inputs

```json
{
  "verdict": "The base holds up on stability and speed but fails on inputs. I found 11 input-driven breaks: 2 major, 9 minor. No blockers.\n\nWhat held, all measured [M]:\n- No panics in 400,000 random inputs; the slowest took 51 us.\n- At 256 bytes or less, the worst per-keystroke cost is 0.21 ms for the base and 0.32 ms for pipeline.\n- A 100,000-digit paste and 10,000 nested parens cost under 1 us in the base.\n- xterm, x11, 3x, 12 x, 0x, 7zip, 7-zip, 1password, 2048, 0ad, c++ and office 365 stay silent.\n- 0.1+0.2 and 1/3*3 are exact. 10/0 and sqrt(-1) are silent.\n- The grafts fix base outputs I confirmed: --5 gives a card, .5+.5 echoes `.5 + .5`, and 10^39+1-10^39 shows 0.\n\nCautions, measured [M]:\n- Pipeline costs 151 ms on a 100,000-digit paste because it has no byte cap before normalizing. The base's pre-cap must stay ahead of the BigRational graft.\n- Minimal aborts the process with a stack overflow on 10,000 nested parens. Graft only its literal echo.\n\nThe 2 major breaks:\n- `0x10` gives a confident `0` card.\n- Default clap parsing rejects every query that starts with a minus. That is the design's own test surface, and clap's usage-error exit code 2 is the same code the design reserves for a stale snapshot.\n\nWhere the files are:\n- Probes and expectation fixtures: /tmp/calc-proto/review-inputs/tests/adversarial_probe.rs, adversarial_expect.rs (16 of 18 fail on the base), adversarial_fuzz.rs and stack_probe.rs.\n- Latency bench: /tmp/calc-proto/review-inputs/examples/adversarial_bench.rs.\n- CLI mock: /tmp/calc-proto/review-inputs/cli-probe/src/main.rs.\n- Units probe: /tmp/calc-proto/review-inputs/spike-probe/src/main.rs.\n- Cap cost: /tmp/calc-proto/review-inputs/cap-variant/examples/scale.rs.\n- Logs: /tmp/calc-proto/review-inputs/probe.log and expect.log.",
  "issues": [
    {
      "title": "Hex literals made only of decimal digits turn into `0 × n` cards",
      "severity": "major",
      "scenario": "Typing `0x10` shows `0 × 10 | Product | 0 | 0 | Zero`, and Enter copies `0` [M]. `0X10`, `0x7` and `0x0` do the same [M]. `0x10+1` shows `1` [M]. Typing `0x1F` flashes the `0` card at the `0x1` keystroke [M]. All three prototypes behave this way [M]. The design's reason for its base covers only the bare `0x` keystroke.",
      "evidence": "Failing fixtures hex_one_digit, hex_two_digits, hex_upper and hex_in_sum in /tmp/calc-proto/review-inputs/tests/adversarial_expect.rs (expect.log) [M]. In quantity/src/lex.rs:22-29 the lexer reads `0` as a number and then `x` as its own word [S]. quantity/src/ops.rs:43-49 lists `x` as a spelling of multiply [S]. TinyCast treats radix prefixes as tokens (calc:74). Its base-conversion stage runs before typed arithmetic (calc:56), and `0xff -` still reads as Hexadecimal (calc:479) [S]. The design's 'Where later features register' list has no base-conversion entry [S].",
      "fix": "Make the tokenizer read `0x`/`0X` (also `0b` and `0o`), with no space, plus the alphanumeric run after it, as one `Tok::Radix` token. The MVP parser rejects that token, so the query is silent. A later base-conversion row consumes it. Add fixtures: `0x1 => ∅`, `0x10 => ∅`, `0x10+1 => ∅`, `0 x 10 => 0 × 10 | Product | 0 | 0 | Zero`, `10x10 => 10 × 10 | Product | 100 | 100 | One Hundred`. Add base conversion to 'Where later features register'."
    },
    {
      "title": "CLI rejects queries that start with a minus, and exit code 2 means two things",
      "severity": "major",
      "scenario": "I tested a clap 4.6.7 derive mock with default settings [M]. `lyncast query -5+3 --json` fails with `error: unexpected argument '-5' found`. `lyncast query --5 --json`, `lyncast query '-(2+3)' --json` and `lyncast activate --query -5+3 --index 0` fail the same way. So the design's fixture `--5 gives ∅` and the shared fixtures `-5+3` and `-(2+3)` cannot run on the CLI test surface. clap exits with code 2 on any usage error. The design reserves exit 2 for 'snapshot differs'. A currency activation test that asserts exit 2 would pass even when its arguments never parsed.",
      "evidence": "Mock CLI: /tmp/calc-proto/review-inputs/cli-probe/src/main.rs. I measured the outputs before and after the fix [M]. clap's usage-error exit code is set in ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/util/mod.rs:32 (`USAGE_CODE: i32 = 2`) [S]. With `allow_hyphen_values = true` on the positional and on `--query`, all four commands parse [M]. Inferred: any getopt-style parser has the same problem [I].",
      "fix": "Set `allow_hyphen_values = true` on `query <text>` and on `activate --query`. Give the snapshot-mismatch failure its own exit code, such as 3, and document it in the CLI contract. Add CLI fixtures for `-5+3`, `--5` and `-(2+3)`."
    },
    {
      "title": "Approximate values show f64 noise as exact integer digits and words",
      "severity": "minor",
      "scenario": "`√2*√2*1000000000000000` shows `2,000,000,000,000,001` with the badge `Two Quadrillion One` [M]. The true answer is 2e15. `√2*1000000000000000000` shows `1,414,213,562,373,095,200`, but the true value is 1,414,213,562,373,095,048.8 [M]. `√99999999999999999999999999999999999` shows `316,227,766,016,837,950`, but the true value is ...837,933.2 [M].",
      "evidence": "quantity/src/format.rs:59-64 keeps max(12, exp+1) digits for every value. quantity/src/format.rs:107-116 spells out any integer left after rounding [S]. The design adopts both rules ('never fewer than the integer digits', 'integers only, after rounding'). Pipeline prints every digit only for exact integers (pipeline/src/format.rs:12-14, 48-52) [S], and it shows `2e+15` here [M]. I computed the true values with Python Decimal at 50 digits [M]. Probe: probe_extra in tests/adversarial_probe.rs.",
      "fix": "Apply 'never fewer than the integer digits' to `Num::Exact` only. Round `Num::Approx` to at most 15 significant digits, padded with zeros in the plain range. Show the target badge only when the rounded value has 15 or fewer significant digits. Add the fixture `√2*√2*1000000000000000 => ... | 2,000,000,000,000,000 | 2000000000000000 | Two Quadrillion`."
    },
    {
      "title": "f64 rounding error gets past the Silent and Domain rules",
      "severity": "minor",
      "scenario": "`√2×√2` shows `2`, but `√2×√2−2` shows `4.4408920985e-16` [M]. `1/(√2×√2−2)` shows `2,251,799,813,685,248` instead of staying silent as division by zero [M]. `√(2−√2×√2)` is silent instead of showing `0` [M]. `√(√2×√2−2)` shows `2.10734242554e-8` [M].",
      "evidence": "quantity/src/quantity.rs:79-86 (`lift` falls back to f64), :97-102 (the zero test reads the f64) and :108-115 (sqrt of a tiny negative gives NaN, so the query is silent) [S]. Pipeline and minimal give the same values [M]. TinyCast computes in Double, so it probably shares this [I].",
      "fix": "After an add or subtract that involves `Num::Approx`, snap the result to `Exact(0)` when |a +/- b| <= 4 * f64::EPSILON * max(|a|, |b|). Do this before the DivideByZero and Domain checks. Pin `√2×√2−2 => ... | 0 | 0 | Zero`, `1/(√2×√2−2) => ∅` and `√(2−√2×√2) => ... | 0 | 0 | Zero`."
    },
    {
      "title": "Exact decimals lose digits in display and copy",
      "severity": "minor",
      "scenario": "`123456789012.34+0` shows and copies `123456789012`, so the cents are gone even though the value is an exact finite decimal [M]. `999999999999.5+0` shows `1,000,000,000,000` with the badge `One Trillion` [M].",
      "evidence": "quantity/src/format.rs:7 (`Sig(12)`) and :59-64 [S]. Pipeline and minimal give the same output [M]. Failing fixture: exact_cents_kept in tests/adversarial_expect.rs.",
      "fix": "For `Num::Exact` values whose decimal expansion ends within 21 significant digits, show every digit. Round to 12 digits only for repeating decimals and `Num::Approx`. The words badge stays integers-only. Add the fixture `123456789012.34+0 => ... | 123,456,789,012.34 | 123456789012.34 | (no words)`."
    },
    {
      "title": "The 256-byte cap counts bytes, so ordinary inputs go silent",
      "severity": "minor",
      "scenario": "A pasted sum of 33 prices (`10.00 + 11.01 + ...`, 261 bytes) is silent [M]. `2` followed by 90 copies of `×2` is 181 characters but 271 bytes, and it is silent [M]. `×` takes 2 bytes, and `√` and U+2212 take 3 each.",
      "evidence": "quantity/src/lib.rs:49 and :57 [S]. calculator.md states no length limit [S]. I raised the cap to 4096 bytes in /tmp/calc-proto/review-inputs/cap-variant. A 1,023-byte sum then costs 0.44 ms median and 0.66 ms max. A 4,095-byte sum costs 1.8 ms median [M].",
      "fix": "Cap at 1024 bytes, or at 256 tokens behind a 1024-byte pre-cap, and keep the depth cap at 32. Inferred: a left-leaning chain of 512 terms recurses 512 deep in eval and echo, which fits any thread stack [I]. Add fixtures: the 33-price sum gives a card, and a 100,000-digit paste stays ∅ in under 1 ms."
    },
    {
      "title": "Text the engine prints cannot be typed back in",
      "severity": "minor",
      "scenario": "The card echoes `1,000 × 1,000`. Typing that echo back is silent [M]. The copy for `99999999999999999999*99999999999999999999` is `1e40`, and `1e40*2` is silent [M]. Once Ctrl+Enter chaining puts the answer back in the query, every answer shown in scientific notation stops the chain.",
      "evidence": "echo_with_commas_is_not_retypeable in tests/adversarial_fuzz.rs, plus grouped_echo_retyped and scientific_copy_pasted in tests/adversarial_expect.rs [M]. TinyCast reads `1,000 + 234` (calc:457) and `1e5` (calc:297-300). Its chaining step makes the answer the next query (calc:620) [S].",
      "fix": "Lex comma groups of exactly 3 digits (`d{1,3}(,ddd)+`) as one number, per calc:457. Lex `<digits>e[+-]<digits>` as a literal, per calc:297. Add fixtures: `1,000 × 1,000 => 1,000,000`, `1e40*2 => 2e40`, `10,5 => ∅`."
    },
    {
      "title": "Full-width characters from CJK input methods are silent",
      "severity": "minor",
      "scenario": "`（2+3）*4`, `1＋2`, `１２×３４` and `(2+3)＊4` are all silent in all three prototypes [M]. A CJK input method in full-width punctuation mode types `（` and `）` [I].",
      "evidence": "quantity/src/lex.rs:33-36 rejects any symbol not in the operator table [S]. Pipeline's normalize step folds only characters in its own table (pipeline/src/normalize.rs:7-16) [S], so it is silent too [M]. TinyCast rewrites the query into canonical form before anything else reads it (calc:16-18) [S].",
      "fix": "Add a canonicalizing pass before tokenizing. It maps U+FF01..U+FF5E to ASCII and U+3000 to a space. Add fixtures `（2+3）*4 => (2 + 3) × 4 | Product | 20 | 20 | Twenty` and `１２×３４ => 408`."
    },
    {
      "title": "Dates and app names get a card that takes over Enter",
      "severity": "minor",
      "scenario": "`2026-10-14` shows `2,026 − 10 − 14 | Difference | 2,002 | 2002 | Two Thousand Two`, and Enter copies `2002` [M]. `10/14/2026` shows `0.000352559582569` [M]. When the planned date pre-token stage lands, the same text gets a different answer, so any MVP fixture or habit built on it changes. `8x8` (I believe this is the 8x8 Work app [I]) shows `64` at index 0, so Enter copies instead of launching.",
      "evidence": "probe_app_names in tests/adversarial_probe.rs [M]. TinyCast parses dates first (calc:51), and its date difference grammar counts an ISO date as a date signal (calc:111) [S]. It puts the card at index 0 (palette.md:13-16) and multiplies `3x3` (calc:431) [S]. So `8x8 => 64` matches TinyCast.",
      "fix": "Until the date stage exists, make `dddd-dd-dd` and `dd/dd/dddd` shapes silent in the pre-token slot, and pin both as fixtures. Ask Hongyi whether an exact app-title match should outrank the card. If not, pin `8x8 => 64` as intended parity."
    },
    {
      "title": "The fixture '1 m + 2 gives 3 m' conflicts with the planned Mismatch error card",
      "severity": "minor",
      "scenario": "Adding quantities requires equal dimensions, so `1 m + 2` is a Mismatch. The quantity spike returns ∅ for it [M]. Once Mismatch becomes `Cannot convert X to Y.`, typing `1 km + 500 m` shows an error card on every keystroke from `1 km + 5` to `1 km + 500 ` [I].",
      "evidence": "quantity/src/quantity.rs:165-171 returns None when the dimensions differ [S]. Running /tmp/calc-proto/review-inputs/spike-probe gives `1 m + 2 -> None` and `--5 km -> −(−5) km | Product` [M]. TinyCast says a bare number takes the unit it is written against (calc:204). It shows errors only for input that can only be a mistake (calc:270) [S].",
      "fix": "Specify a scalar-adoption rule for `+` and `-` in `Apply`: a dimensionless operand takes the other operand's unit before the dimension check. Keep adjacency silent (`1hr 30`), per calc:204-207. Add `1 km + 5 => 6 km` as a units fixture."
    },
    {
      "title": "The JSON chord spelling differs from the grafted code",
      "severity": "minor",
      "scenario": "The design's byte-for-byte row uses `\"chord\":\"enter\"`. The row printer it grafts emits `\"chord\":\"Enter\"`. Grafting it unchanged fails the JSON fixture.",
      "evidence": "pipeline/src/present.rs:78 [S]. The design's Grafts list, item 3, names that file and lines 72-86.",
      "fix": "Emit the chord name from the one chord table. Do not hard-code a string in the row printer. Pin the chord spelling in a fixture."
    }
  ]
}
```
