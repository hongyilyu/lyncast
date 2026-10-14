# The calculator is one pure function over the root query

This page explains how the calculator turns root search text into a card, why the quantity prototype won the bake-off, and where later features plug in. The [decision log](../decisions.md#calculator-engine) wins any disagreement. Evidence is in [notes/08](../research/notes/08-calc-engine-workflow.md) unless a link says otherwise, and "the base" means the quantity prototype.

## The engine answers every root query with an outcome

Every root feature evaluates a plain query, and the section table puts Answer first, then Apps, then Fallback ([decision log](../decisions.md#input-box)). The calculator's share is one pure call per keystroke:

```rust
calc::evaluate(text, &Env) -> Outcome
```

`Card(Answer)` puts a card at index 0. `Silent` leaves index 0 to the apps. `Error` waits for error cards, and the MVP never returns it. The parser alone decides that `12x34` is math and `firefox` is not.

## Ten steps turn the text into an outcome

`evaluate` runs these steps. A step that fails returns `Silent`.

1. **Canonicalize.** Trim, then fold U+FF01 to U+FF5E to ASCII and U+3000 to a space. All three prototypes were silent on `（2+3）*4` (measured).
2. **Cap.** Text over 1024 bytes is silent. A 256-byte cap silenced a pasted sum of 33 prices, and a 1,023-byte sum costs 0.66 ms at worst (measured).
3. **Gate.** Empty text, one ASCII word, and the date shapes `dddd-dd-dd` and `dd/dd/dddd` are silent. Ungated, `2026-10-14` showed `Difference 2,002` (measured).
4. **Tokenize once.** Numbers, words, parentheses, and the operator table's symbols become tokens. `0x`, `0b`, or `0o` followed by digits is one radix token, and the MVP is silent on it. Without the rule, all three prototypes read `0x10` as `0 × 10` (measured).
5. **Strip one trailing operator.** Only an infix symbol counts, so `12 x` stays a search and `10 + -` is silent.
6. **Pratt parse.** An operand is a prefix row, a number, a closed group, or a catalog name, and `catalog::lookup(word)` finds nothing yet. A `(` or a root after an operand implies `×`, so `4(2+3)` is 20, but `5 3` is silent. Leftover tokens, open parentheses, and nesting past 32 are silent. Without the depth cap, 126 nested parentheses overflowed a 256 KB thread (measured). Parsing never reads `Env`.
7. **Reject a bare number.** A number, negated any number of times, is silent unless step 5 stripped an operator. `--5` is silent, but `10 +` shows `10`.
8. **Evaluate with `Env`.** `eval(&Expr, &Env) -> Result<Quantity, EvalError>` calls each row's `apply`. Atoms get their values here, so a missing rate can later become an error card.
9. **Format.** Round once, then render display, copy, and words from the same digits.
10. **Return the `Answer`.** Wrap it in `Outcome::Card`. In the MVP, every `EvalError` becomes `Silent`.

## The types carry an empty dimension

The sketch merges the base, the judge's grafts, and the decision log:

```rust
pub fn evaluate(text: &str, env: &Env) -> Outcome;
pub enum Outcome { Card(Answer), Error(String), Silent }
pub struct Env<'a> { pub rates: Option<&'a Rates> } // None in the MVP
pub enum Num { Exact(Ratio), Approx(f64) }          // Ratio holds i128 parts
pub struct Dim(pub [i8; BASES]);                    // BASES = 0 in the MVP
pub struct Quantity { num: Num, dim: Dim }
pub enum EvalError { DivideByZero, Domain, Overflow, Mismatch, NoRate(Code), NoSnapshot }
pub struct Answer {
	expression: String,
	source_badge: String,
	display: String,
	copy: String,
	target_badge: Option<String>,
}
```

The judge grafted `BigRational`, but the decision log allows no big-number crates, so an `i128` overflow is silent. The base fell back to `f64` without a signal and showed `0` for `10^39+1-10^39` (measured). No card beats a confident wrong one.

## One operator table owns every operator

Each row owns its spellings, binding power, echo glyph, badge, and `apply` function. A new operator is a new row.

| Row | Spellings | Echo | Badge |
|---|---|---|---|
| `ADD` | `+` | `+` | Sum |
| `SUB` | `-`, `−` | `−` | Difference |
| `MUL` | `*`, `×`, `·`, `x` | `×` | Product |
| `DIV` | `/`, `÷` | `÷` | Quotient |
| `NEG` | `-`, `−` | `−` | Negation |
| `SQRT` | `√`, `sqrt`, `square root of` | `√` | Square Root |

`×` and `÷` bind tighter than `+` and `−`, and the prefix rows `NEG` and `SQRT` bind tightest. `x` is a word, so it means `×` only after an operand.

## MVP rules fix badges, digits, and silence

**Badges.** The source badge is the root row's badge, or `Result` for a lone number after a stripped operator, as in TinyCast. The words badge spells whole numbers only ([lead default](../decisions.md#lead-defaults-hongyi-can-override)), in title-case short scale with hyphenated tens and `Minus` for negatives, and never in scientific notation. No decision fixes its length cap. The base used 64 characters, and the judge proposed 40.

**Digits.** Display groups digits with commas, and copy drops them. Exact decimals that terminate keep every digit. The base lost the cents of `123456789012.34+0` (measured). Other values round to 12 significant digits, but never fewer than their integer digits, in plain notation for exponents -7 to 20. Approximate values stop at 15 significant digits, because the base showed `√2*√2*1000000000000000` as `2,000,000,000,000,001` (measured).

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

- **Units.** Rows go in `units.rs`, `catalog::lookup` asks units first, and `BASES` grows. A `CONVERT` row in `ops.rs` spells `to`, `->`, and `→` at the lowest binding power. A unit binds to the operand before it, tighter than `×`, as `Expr::Measure` did in the minimal spike. The unit type must compose, so that `km/h` has a name.
- **Currency.** Codes join the catalog after units, and `eval` prices them from `Env.rates`. `NoSnapshot`, `NoRate(code)`, and `Mismatch` then become error cards, such as `No exchange rate for <CODE>.` Rows name their snapshot, and activation against a stale one exits 3.
- **Dates and time zones.** A pre-token stage replaces the date-shape gate and reads an injected `Env.now`. The single-word gate gains `now`, `today`, `tomorrow`, `yesterday`, and `time`. The integration review asks rows to pin the clock and zone, or `time` can show 10:41 and copy 10:42 (inferred).
- **Base conversion.** A stage before typed arithmetic consumes the radix token, as in TinyCast.
- **Error cards.** `Outcome::Error` and `EvalError` exist now, so error cards change no signatures. The error row's kind and Enter action are not chosen yet.

## Fixtures pin every rule as literal output

Each fixture line reads `input => echo | badge | display | copy | words`, and ∅ means no card.

### The 26 shared fixtures pass in every prototype

All three prototypes passed these 26 inputs (measured). notes/08 records only the count, so the rows come from the base's scratch fixture file.

```text
12×34 => 12 × 34 | Product | 408 | 408 | Four Hundred Eight
2+3*4 => 2 + 3 × 4 | Sum | 14 | 14 | Fourteen
(2+3)*4 => (2 + 3) × 4 | Product | 20 | 20 | Twenty
10/4 => 10 ÷ 4 | Quotient | 2.5 | 2.5 | none
7-10 => 7 − 10 | Difference | -3 | -3 | Minus Three
-5+3 => −5 + 3 | Sum | -2 | -2 | Minus Two
0.1+0.2 => 0.1 + 0.2 | Sum | 0.3 | 0.3 | none
1/3*3 => 1 ÷ 3 × 3 | Product | 1 | 1 | One
sqrt(16) => √16 | Square Root | 4 | 4 | Four
√(9+16) => √(9 + 16) | Square Root | 5 | 5 | Five
sqrt 2 => √2 | Square Root | 1.41421356237 | 1.41421356237 | none
1000*1000 => 1,000 × 1,000 | Product | 1,000,000 | 1000000 | One Million
```

`12*34`, `12x34`, `12 x 34`, and `12×34+` give the `12×34` card, and `√16` gives the `sqrt(16)` card. `(2+3`, `12`, `firefox`, `sqrt`, `5 3`, `1.2.3+1`, `10/0`, `sqrt(-1)`, and the empty string give ∅.

### The decision log adds fixtures for its own rules

The rules above complete the rows that the notes give only in part.

```text
0 x 10 => 0 × 10 | Product | 0 | 0 | Zero
1920x1080 => 1,920 × 1,080 | Product | 2,073,600 | 2073600 | words depend on the cap
（2+3）*4 => (2 + 3) × 4 | Product | 20 | 20 | Twenty
10 + => 10 | Result | 10 | 10 | Ten
123456789012.34+0 => 123,456,789,012.34 + 0 | Sum | 123,456,789,012.34 | 123456789012.34 | none
√2*√2*1000000000000000 => √2 × √2 × 1,000,000,000,000,000 | Product | 2,000,000,000,000,000 | 2000000000000000 | Two Quadrillion
```

`１２×３４` gives the `12×34` card. A pasted 261-byte sum of 33 prices gives a card. These inputs give ∅:

- `0x1`, `0x10`, and `0x10+1` are radix tokens.
- `0x`, `12 x`, and `3x` end in a word.
- `xterm` and `7zip` stayed silent on the base (measured).
- `2026-10-14` and `10/14/2026` are date shapes.
- `--5` is a bare number, and `10 + -` ends in two operators.
- A 100,000-digit paste and 100,000 nested parentheses exceed the cap and return in under 1 ms.

### Hyphen queries reach the engine

Default clap rejected `lyncast query -5+3` (measured), so the query text sets `allow_hyphen_values`.

- `lyncast query -5+3` prints the `-5+3` card.
- `lyncast query --5` prints no calculator row.
- `lyncast query '-(2+3)'` prints `−(2 + 3) | Negation | -5 | -5 | Minus Five`.
- A usage error exits 2.
