# The calculator is one pure function over the root query

This page explains how the calculator turns root search text into a card, why the quantity prototype won the bake-off, and where later features plug in. The [decision log](../decisions.md#calculator-engine) wins any disagreement. Evidence is in [notes/08](../research/notes/08-calc-engine-workflow.md) unless a link says otherwise, and "the base" means the quantity prototype.

## The engine answers every root query with an outcome

Every root feature evaluates a plain query, and the section table puts Answer first, then Results ([decision log](../decisions.md#row-shapes)). The calculator's share is one pure call per keystroke:

```rust
calc::evaluate(text, &Env) -> Outcome
```

`Card(Answer)` puts a card at index 0. `Silent` leaves index 0 to the apps. `Error` waits for error cards, and the MVP never returns it. The parser alone decides that `12x34` is math and `firefox` is not.

## Ten steps turn the text into an outcome

`evaluate` runs these steps. A step that fails returns `Silent`.

1. **Canonicalize.** Trim, then fold U+FF01 to U+FF5E to ASCII and U+3000 to a space. All three prototypes were silent on `（2+3）*4` (measured).
2. **Cap.** Text over 1024 bytes is silent. A 256-byte cap silenced a pasted sum of 33 prices, and on the base a 1,023-byte sum cost 0.66 ms at worst (measured). The port's worst 1024-byte input takes 0.6 ms (measured, below). Folding shrinks text by at most 3 to 1, so raw text over 3072 bytes is silent before it is folded, and an oversized paste costs one length check.
3. **Gate.** Empty text, one ASCII word, and text that holds a date shape `dddd-dd-dd` or `dd/dd/dddd` anywhere are silent. Ungated, `2026-10-14` showed `Difference 2,002` (measured). The date stage will read the whole query, so `2026-10-14+1` and `2026-10-145` are silent too. A minus sign counts as a hyphen, because `−` spells subtraction.
4. **Tokenize once.** Numbers, words, parentheses, and the operator table's symbols become tokens. `0x`, `0b`, or `0o` followed by letters or digits is one radix token, and the MVP is silent on it. Without the rule, all three prototypes read `0x10` as `0 × 10` (measured).
5. **Strip one trailing operator.** Only an infix symbol counts, so `12 x` stays a search and `10 + -` is silent.
6. **Pratt parse.** An operand is a prefix row, a number, a closed group, or a catalog name, and `catalog::lookup(word)` finds nothing yet. A `(` after an operand implies `×`, so `4(2+3)` is 20, but `5 3` is silent. Leftover tokens, open parentheses, and nesting past 32 are silent. Nesting counts parentheses and prefix operators, so 32 nested groups parse and 33 do not. Without the depth cap, 126 nested parentheses overflowed a 256 KB thread in a release build of the base (measured, notes/08). Operators of one binding power form one chain, a list of terms, so only groups and prefix operators nest. With the base's binary tree, a 512-term sum needed 1 MB of stack in a debug build of the port and 192 KB in release, measured before the port switched to chains. With chains, the deepest input under both caps, `1+2×(` nested 32 deep, needs 148 KB and 44 KB (measured). Parsing never reads `Env`.
7. **Reject a bare number.** A number, negated any number of times, is silent unless step 5 stripped an operator. `--5` is silent, but `10 +` shows `10`.
8. **Evaluate with `Env`.** `eval(&Expr, &Env) -> Result<Quantity, EvalError>` calls each row's `apply`. Atoms get their values here, so a missing rate can later become an error card.
9. **Format.** Round once, then render display, copy, and words from the same digits.
10. **Return the `Answer`.** Wrap it in `Outcome::Card`. In the MVP, every `EvalError` becomes `Silent`.

## The types carry an empty dimension

The sketch merges the base, the judge's grafts, and the decision log. `lyncast-core::calc` makes public only the first four items:

```rust
pub fn evaluate(text: &str, env: &Env) -> Outcome;
pub enum Outcome { Card(Answer), Error(String), Silent }
pub struct Env {}                                   // gains the rates with currency
pub struct Answer { /* private, so only the engine builds one */ }
impl Answer {
	pub fn expression(&self) -> &str;
	pub fn source_badge(&self) -> &str;
	pub fn display(&self) -> &str;
	pub fn copy(&self) -> &str;
	pub fn target_badge(&self) -> Option<&str>;
}

struct Num { n: i128, d: i128 }                     // a fraction in lowest terms
struct Dim([i8; BASES]);                            // BASES = 0 in the MVP
struct Quantity { num: Num, dim: Dim }
enum EvalError { DivideByZero, Overflow, Mismatch }
enum Name {}                                        // what catalog::lookup finds; none yet
```

The judge grafted `BigRational`, but the decision log allows no big-number crates, so an `i128` overflow is silent. That covers a literal too long for `i128`, and a step along the way: `(2^127−1)/2 + 1/2` is silent, although the reduced answer fits. Only wider integers would avoid that. The base fell back to `f64` without a signal and showed `0` for `10^39+1-10^39` (measured). No card beats a confident wrong one.

The MVP carries only the slots it can fill. `catalog::lookup(word) -> Option<Name>` takes no `Env`, and `eval` prices a name with `Env`, so the signatures keep parsing away from `Env`. `Name` has no variants, so units and currencies add them, and the compiler finds every place that handles one. `Rates`, `NoRate(code)`, and `NoSnapshot` arrive with currency, which fixes their shapes. The base's `Unit` and `Precision` are gone, because a single optional unit cannot name `km/h`.

The MVP has no square root ([decision log](../decisions.md#calculator-engine)), so every value is an exact fraction and nothing is approximate. With `f64` roots, `√2×√2−2` showed `4.4408920985e-16` and `√2+10000000000000000−10000000000000000` showed `2` (measured). `√`, `sqrt`, and `square root of` now spell nothing, so a query that uses them is silent.

## One operator table owns every operator

Each row owns its spellings, binding power, echo glyph, badge, and `apply` function. A new operator is a new row.

| Row | Spellings | Echo | Badge |
|---|---|---|---|
| `ADD` | `+` | `+` | Sum |
| `SUB` | `-`, `−` | `−` | Difference |
| `MUL` | `*`, `×`, `·`, `x` | `×` | Product |
| `DIV` | `/`, `÷` | `÷` | Quotient |
| `NEG` | `-`, `−` | `−` | Negation |

`×` and `÷` bind tighter than `+` and `−`, and the prefix row `NEG` binds tightest. `x` is a word, so it means `×` only after an operand. Word spellings match without case, so `12X34` is `12 × 34`.

The lexer reads each token against the table once and hands the parser the rows it can mean, so `-` arrives as both `SUB` and `NEG`. The parser never reads a spelling. The base matched every spelling again at each parser step.

## MVP rules fix badges, digits, and silence

**Badges.** The source badge is the root row's badge, or `Result` for a lone number after a stripped operator, as in TinyCast. The words badge spells whole numbers only ([lead default](../decisions.md#lead-defaults-hongyi-can-override)), in title-case short scale with hyphenated tens and `Minus` for negatives, and never in scientific notation. Whole means whole as displayed, after rounding, so `2999999999999/3` reads `One Trillion`. Words longer than 40 characters are dropped ([decision log](../decisions.md#calculator-engine)), so `1920x1080` shows no words. A 64-character cap would have kept words on every number below a million.

**Echo.** The echo shows the query as the engine read it: spaced operators, the table's echo glyphs, grouped integer digits, and only the parentheses that precedence needs. Literals are canonical too, so `.5` echoes `0.5`, `007` echoes `7`, and `5.` echoes `5`. Fraction digits stay as typed, so `10.50` echoes `10.50`.

**Digits.** Display groups digits with commas, and copy drops them. Exact decimals that terminate keep every digit, in plain notation at any size. The base lost the cents of `123456789012.34+0` (measured). Repeating decimals round to 12 significant digits, but never fewer than their integer digits, so they print plain at any size and switch to scientific notation only below 10^-7.

**Silence.** Every MVP failure is silent, including division by zero, because `10/0` is a prefix of `10/0.5` ([lead default](../decisions.md#lead-defaults-hongyi-can-override)).

**`NxN` is math.** Hongyi decided that `1920x1080` shows 2,073,600, although look's corpus treats it as a search ([notes/09](../research/notes/09-input-box-prior-art.md)). An app with that name sits one row below the card, as with `8x8`. The radix rule comes first, so `0x10` is silent.

## The daemon builds the row

The engine knows no verbs. The daemon turns a `Card` into a row:

- `key` is `calc:<raw query>`, because `10 +` echoes `10` and a bare `10` is silent.
- `kind` is Calculator, `index` is 0, and `section` is Answer.
- `actions` come from the chord table in [the clipboard and paste design](clipboard-and-paste.md). Enter and Ctrl+Enter both copy in the MVP.
- The other fields come from `Answer`.

Activation evaluates the key's text again ([architecture](architecture.md)). The MVP reads nothing from `Env`, so the copy matches the card, and `lyncast activate 'calc:2+3*4'` prints `copied 14` ([notes/07](../research/notes/07-paste-seam-workflow.md)).

## The quantity design won on extension cost

Three prototypes built the same MVP and the same units and currency spike, and all passed the 26 shared fixtures. The judge re-ran the numbers below, except the score (measured).

| | quantity | minimal | pipeline |
|---|---|---|---|
| Old-file lines the spike changed | +24, -5 in 6 files | +124, -20 in 4 of 4 | +83, -33 in 9 of 11 |
| Time per evaluation | 1.13 µs | 622 ns | 1.69 µs |
| 100,000-digit paste | 0 µs | 109 µs | 202,618 µs |
| 100,000 nested parentheses | 0 µs | stack overflow abort | 2,805 µs |
| `0x` and `12 x` | silent | card | card |
| Judge's score | 29 | 24 | 24 |

Quantity paid for its slots up front. A control without them has 756 lines to quantity's 901 and is no faster, and the spike left the tokenizer and the Pratt loop alone. Units and currency arrive in this engine as new kinds of quantity, so extension cost is what counts. Minimal, at 565 lines, is smaller and faster, but it has no cap or depth limit.

The +24 and -5 figure flatters quantity. Its spike treated a unit as an implied product, which the final design rejects, so `5 km / 1 km` gives no card (measured). The integration reviewer infers that real units will touch all 7 source files. The ranking still stands, because the other two paid more for the simpler spike.

## Later features register in known places

- **Units.** Rows go in `units.rs`, `catalog::lookup` asks units first, and `BASES` grows. A `CONVERT` row in `ops.rs` spells `to`, `->`, and `→` at the lowest binding power. A unit binds to the operand before it, tighter than `×`, as `Expr::Measure` did in the minimal spike. The unit type must compose, so that `km/h` has a name. The parser has no rule yet for how a name binds to the operand before it, and the compiler points at that place once `Name` has a value.
- **Currency.** Codes join the catalog after units, and `eval` prices them from `Env.rates`. `NoSnapshot`, `NoRate(code)`, and `Mismatch` then become error cards, such as `No exchange rate for <CODE>.` Rows name their snapshot, and activation against a stale one exits 3.
- **Dates and time zones.** A pre-token stage replaces the date-shape gate and reads an injected `Env.now`. The single-word gate gains `now`, `today`, `tomorrow`, `yesterday`, and `time`. The integration review asks rows to pin the clock and zone, or `time` can show 10:41 and copy 10:42 (inferred).
- **Base conversion.** A stage before typed arithmetic consumes the radix token, as in TinyCast.
- **Error cards.** `Outcome::Error` and `EvalError` exist now, so error cards change no signatures. The error row's kind and Enter action are not chosen yet.

## Fixtures pin every rule as literal output

Each fixture line reads `input => echo | badge | display | copy | words`. ∅ means no card, and `none` means no words. An input in double quotes, such as `""`, is read without the quotes.

`crates/lyncast-core/tests/calc.rs` reads every `fixtures` block on this page and fails on any line it cannot read, so this page is the only copy of the fixtures.

### The 26 shared inputs

All three prototypes passed these 26 inputs (measured). notes/08 records only the count, so the rows come from the base's scratch fixture file. Square root left the MVP on 2026-10-15, so the four root inputs that gave cards now give ∅.

```fixtures
12×34 => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
12*34 => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
12x34 => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
12 x 34 => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
12×34+ => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
2+3*4 => 2 + 3 × 4 | Sum | 14 | 14 | Fourteen
(2+3)*4 => (2 + 3) × 4 | Product | 20 | 20 | Twenty
10/4 => 10 ÷ 4 | Quotient | 2.5 | 2.5 | none
7-10 => 7 − 10 | Difference | -3 | -3 | Minus Three
-5+3 => −5 + 3 | Sum | -2 | -2 | Minus Two
0.1+0.2 => 0.1 + 0.2 | Sum | 0.3 | 0.3 | none
1/3*3 => 1 ÷ 3 × 3 | Product | 1 | 1 | One
sqrt(16) => ∅
√16 => ∅
√(9+16) => ∅
sqrt 2 => ∅
1000*1000 => 1,000 × 1,000 | Product | 1,000,000 | 1000000 | One Million
(2+3 => ∅
12 => ∅
firefox => ∅
sqrt => ∅
5 3 => ∅
1.2.3+1 => ∅
10/0 => ∅
sqrt(-1) => ∅
"" => ∅
```

### The decision log adds fixtures for its own rules

The rules above complete the rows that the notes give only in part.

```fixtures
0 x 10 => 0 × 10 | Product | 0 | 0 | Zero
1920x1080 => 1,920 × 1,080 | Product | 2,073,600 | 2073600 | none
（2+3）*4 => (2 + 3) × 4 | Product | 20 | 20 | Twenty
１２×３４ => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
10 + => 10 | Result | 10 | 10 | Ten
123456789012.34+0 => 123,456,789,012.34 + 0 | Sum | 123,456,789,012.34 | 123456789012.34 | none
1000+373 => 1,000 + 373 | Sum | 1,373 | 1373 | One Thousand Three Hundred Seventy-Three
3000+323 => 3,000 + 323 | Sum | 3,323 | 3323 | none
0x1 => ∅
0x10 => ∅
0x10+1 => ∅
0x 10 => 0 × 10 | Product | 0 | 0 | Zero
0x(2) => 0 × 2 | Product | 0 | 0 | Zero
8x8 => 8 × 8 | Product | 64 | 64 | Sixty-Four
0X10 => ∅
0b101 => ∅
0o17 => ∅
0x => ∅
12 x => ∅
3x => ∅
xterm => ∅
7zip => ∅
2026-10-14 => ∅
10/14/2026 => ∅
２０２６-１０-１４ => ∅
2026-10-14+1 => ∅
2026-10-145 => ∅
12026-10-14 => ∅
2026−10−14 => ∅
--5 => ∅
10 + - => ∅
1000000000000000000000000000000000000000+1-1000000000000000000000000000000000000000 => ∅
99999999999999999999*99999999999999999999 => ∅
```

These rows pin these rules:

- `1,373` spells in exactly 40 characters and `3,323` in 41, so the pair holds the words cap.
- `0x`, `0b`, and `0o` followed by letters or digits are one radix token. `0x`, `12 x`, and `3x` end in a word, not an operator. `xterm` and `7zip` stayed silent on the base (measured).
- A date shape anywhere in the query is silent, even next to more digits, and a minus sign counts as a hyphen.
- `--5` is a bare number, and `10 + -` ends in two operators.
- A literal or a result past `i128` is silent. The base showed `0` for the 10^39 line and `1e40` for the product (measured).

### The base's own rules carry over

The base pinned these rules in its scratch fixture file. Its root rows left with square root, except `square root of 16`, which now pins that the phrase is silent. Three other rows changed under the decision log. `1234*1000+21` lost its words to the 40-character cap. `99999999999999999999*99999999999999999999` is now ∅, in the block above. `123456789012345+0.5` keeps its `.5`, in the next block. `60*60*24` is new, to keep a long spelling that fits the cap.

```fixtures
10 * - => ∅
10 of => ∅
-5 => ∅
(12) => ∅
4(2+3) => 4(2 + 3) | Product | 20 | 20 | Twenty
(2+3)(2+3) => (2 + 3)(2 + 3) | Product | 25 | 25 | Twenty-Five
6/2(1+2) => 6 ÷ 2(1 + 2) | Product | 9 | 9 | Nine
2(3) => 2(3) | Product | 6 | 6 | Six
square root of 16 => ∅
8 ÷ 2 − 1 · 3 => 8 ÷ 2 − 1 × 3 | Difference | 1 | 1 | One
2*-3 => 2 × −3 | Product | -6 | -6 | Minus Six
10-(2+3) => 10 − (2 + 3) | Difference | 5 | 5 | Five
1/3 => 1 ÷ 3 | Quotient | 0.333333333333 | 0.333333333333 | none
2/3 => 2 ÷ 3 | Quotient | 0.666666666667 | 0.666666666667 | none
60*60*24 => 60 × 60 × 24 | Product | 86,400 | 86400 | Eighty-Six Thousand Four Hundred
1234*1000+21 => 1,234 × 1,000 + 21 | Sum | 1,234,021 | 1234021 | none
0-0 => 0 − 0 | Difference | 0 | 0 | Zero
9007199254740993+0 => 9,007,199,254,740,993 + 0 | Sum | 9,007,199,254,740,993 | 9007199254740993 | none
1/300000000 => 1 ÷ 300,000,000 | Quotient | 3.33333333333e-9 | 3.33333333333e-9 | none
. => ∅
.+1 => ∅
2+3 km => ∅
1+1 2 => ∅
12- => 12 | Result | 12 | 12 | Twelve
0-1373 => 0 − 1,373 | Difference | -1,373 | -1373 | none
4/7 => 4 ÷ 7 | Quotient | 0.571428571429 | 0.571428571429 | none
3/7 => 3 ÷ 7 | Quotient | 0.428571428571 | 0.428571428571 | none
2999999999999999999/300000000000000000000000000 => 2,999,999,999,999,999,999 ÷ 300,000,000,000,000,000,000,000,000 | Quotient | 1e-8 | 1e-8 | none
1e5 => ∅
1,000+1 => ∅
2+2= => ∅
```

### Echo, exactness, and notation

These rows pin the echo and digit rules in the MVP section above. `10^22+1-10^22` is exact past `f64`, where minimal showed `0` (measured). The base rounded `123456789012345+0.5` to `123,456,789,012,346` (measured).

```fixtures
.5+.5 => 0.5 + 0.5 | Sum | 1 | 1 | One
007+5. => 7 + 5 | Sum | 12 | 12 | Twelve
10.50+1 => 10.50 + 1 | Sum | 11.5 | 11.5 | none
20000000000000/3 => 20,000,000,000,000 ÷ 3 | Quotient | 6,666,666,666,667 | 6666666666667 | none
1÷−2 => 1 ÷ −2 | Quotient | -0.5 | -0.5 | none
2999999999999/3 => 2,999,999,999,999 ÷ 3 | Quotient | 1,000,000,000,000 | 1000000000000 | One Trillion
29999999999999/30000000000000 => 29,999,999,999,999 ÷ 30,000,000,000,000 | Quotient | 1 | 1 | One
1/3000000 => 1 ÷ 3,000,000 | Quotient | 0.000000333333333333 | 0.000000333333333333 | none
1/30000000 => 1 ÷ 30,000,000 | Quotient | 3.33333333333e-8 | 3.33333333333e-8 | none
2999999999999999999/30000000000000000000000000 => 2,999,999,999,999,999,999 ÷ 30,000,000,000,000,000,000,000,000 | Quotient | 0.0000001 | 0.0000001 | none
0.00000000000000000001+0.00000000000000000001 => 0.00000000000000000001 + 0.00000000000000000001 | Sum | 0.00000000000000000002 | 0.00000000000000000002 | none
-2*3 => −2 × 3 | Product | -6 | -6 | Minus Six
1000000000000000000000000000000000000+0 => 1,000,000,000,000,000,000,000,000,000,000,000,000 + 0 | Sum | 1,000,000,000,000,000,000,000,000,000,000,000,000 | 1000000000000000000000000000000000000 | One Undecillion
1.5000000000000000000000000000000000000000+1 => 1.5000000000000000000000000000000000000000 + 1 | Sum | 2.5 | 2.5 | none
2(−3) => 2(−3) | Product | -6 | -6 | Minus Six
−(−5)+ => −(−5) | Result | 5 | 5 | Five
1000000000000000000000+0 => 1,000,000,000,000,000,000,000 + 0 | Sum | 1,000,000,000,000,000,000,000 | 1000000000000000000000 | One Sextillion
12X34 => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
10x10 => 10 × 10 | Product | 100 | 100 | One Hundred
10000000000000000000000+1-10000000000000000000000 => 10,000,000,000,000,000,000,000 + 1 − 10,000,000,000,000,000,000,000 | Difference | 1 | 1 | One
123456789012345+0.5 => 123,456,789,012,345 + 0.5 | Sum | 123,456,789,012,345.5 | 123456789012345.5 | none
123456789012345678901234+1 => 123,456,789,012,345,678,901,234 + 1 | Sum | 123,456,789,012,345,678,901,235 | 123456789012345678901235 | none
100000000000000000000000/3 => 100,000,000,000,000,000,000,000 ÷ 3 | Quotient | 33,333,333,333,333,333,333,333 | 33333333333333333333333 | none
```

### Hyphen queries reach the engine

The engine gives these cards. The daemon's [card session](daemon.md#the-calculator-card-comes-first) checks them again through the CLI.

```fixtures
−5+3 => −5 + 3 | Sum | -2 | -2 | Minus Two
-(2+3) => −(2 + 3) | Negation | -5 | -5 | Minus Five
```

Default clap rejected `lyncast query -5+3` (measured), so the query text sets `allow_hyphen_values`. That session and [ranking's](ranking.md#lyncast-query-prints-one-json-object-per-row) `-5+3` cli block check these through the CLI:

- `lyncast query -5+3` prints the `-5+3` card.
- `lyncast query --5` prints no calculator row.
- `lyncast query -(2+3)` prints the card whose `card` object holds `−(2 + 3)`, `Negation`, `-5`, `-5`, and `Minus Five`.
- A usage error exits 2.

### Generated inputs do not fit on one line

`tests/calc.rs` builds these inputs and checks the same outputs:

- 100,000 digits followed by `+1`, and `1+1` inside 100,000 nested parentheses, give ∅.
- `10` followed by 511 copies of `+1` is 1024 bytes and gives 521. Starting with `100` instead, it is 1025 bytes and gives ∅.
- `1+1` followed by 4,000 spaces gives 2, because the text is trimmed before either cap.
- `１＋` repeated 200 times and then `１` is 1,203 bytes, but 401 bytes after folding, and gives 201. Repeated 511 times and then `１１`, it is exactly 3,072 bytes, at the raw cap, and 1,024 after folding, and gives 522.
- `1`, 400 ideographic spaces (U+3000), and `+1` is 1,203 bytes, but 403 after folding, and gives 2.
- `1+1` inside 32 nested parentheses gives 2, and inside 33 gives ∅. 31 minus signs before `(1+1)` give a card, and 32 give ∅, because prefix operators count toward the depth too.
- The pasted column `10.00 + 11.01 + … + 42.32`, 33 prices in 261 bytes, gives 863.28.
- The deepest and longest inputs under both caps give their full cards: a 512-term chain of `*1` and a 341-term chain of `(1)`, each 1,023 bytes, 32 groups around 471 terms, `1+2×(` nested 32 deep (8,589,934,591), 31 minus signs before `(1+1)`, and `2(` nested 32 deep.

The test runs them all on a 256 KB thread, so deeper recursion fails the test instead of a user's palette. `tests/calc_fuzz.rs` evaluates 200,000 seeded random queries with overflow checks on. It fails on any panic, and it checks that at least a fifth of them give cards, so a silent engine cannot pass.

### The port's cost per keystroke

`cargo run --release -p lyncast-core --example calc_bench` times the 26 shared inputs as notes/08 did, then the worst inputs under the caps. On the dev box (AMD EPYC 9R14), as the median of 7 runs, with about 2% spread between processes (measured):

| Input | Time per evaluation |
|---|---|
| The 26 shared inputs | 0.45 µs (with roots, 0.55 µs; the base took 1.12 µs on the same day, with its four root cards) |
| A 100,000-digit paste, or 100,000 nested parentheses | 0.011 µs |
| A 512-term sum, 1023 bytes | 87 µs |
| A Fibonacci fraction followed by 473 copies of `+0`, 1024 bytes | 602 µs |

A sampling profile of the shared inputs shows no hot spot. Allocation takes about a fifth of the time, and parsing, formatting, the spelling lookup, and the echo about a tenth each. The worst input spends 85% of its time in Euclid's gcd, because adjacent Fibonacci numbers need the most steps. Before the lexer told words from symbols by one ASCII byte, a Unicode lookup on `−`, `×`, and `÷` at every token took 15% of the shared inputs' time (sampled).
