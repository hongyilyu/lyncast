# frizbee with TinyCast's points (lead measurement, 2026-10-15)

The decision log picked frizbee for matching and TinyCast's rules for ranking. TinyCast ranks with its own point scorer and a Medium threshold written in those points. This note records how frizbee 0.13.0's scores were made to fit that threshold, and what the fit costs per keystroke. [design/ranking.md](../../design/ranking.md) holds the resulting rules and their fixtures.

## Method

`bench/score-map` is its own cargo workspace. Run it from the repository root with `cargo run --release --manifest-path bench/score-map/Cargo.toml --target-dir target/bench-score-map`, as `scripts/check.sh` builds it, so the build lands in the ignored `target/`.

- **Reference.** A dynamic-programming scorer written from the scorer section of TinyCast's launcher doc. It gives 4 for the query's first character on the text's first, 3 on a word start (after a separator, at a capital after a lowercase letter, at a letter after a digit, or at the last capital of a run before a lowercase letter), 2 for any other match, 1 for a separator on a different separator, and −1 for each gap. It skips query separators that have nothing to land on and takes the best alignment. No TinyCast code was read.
- **Corpus.** 225 names in `bench/score-map/src/names.txt`: 216 app names typical of Linux desktops, then lyncast's nine command titles.
- **Queries.** Every prefix up to 10 characters, word initials, runs of 2 to 5 characters inside each word, and 4 seeded random subsequences per name. That gives 3,451 queries without separators and 290 with them. Every query runs against every name.
- **Candidates.** A is frizbee with its default weights. B is frizbee with TinyCast's points as weights: `match_score` 2, `prefix_bonus` 2, `delimiter_bonus` 1, `capitalization_bonus` 1, `gap_open_penalty` 1, `gap_extend_penalty` 0, `mismatch_penalty` 4, and 0 for the case and exact bonuses, all with `max_typos` 0.

## Results

All numbers are measured, on the dev box (AMD EPYC 9R14).

| Check | A, default weights | B, TinyCast's points |
|---|---|---|
| Aligned pairs where frizbee and the reference disagree on whether a match exists | 0 | 0 |
| Pairs with the same score as the reference | not comparable | 17,323 of 17,596 (98.4%), the rest 1 apart (213 lower, 60 higher) |
| Medium decisions, pairs both show | 8,236 with the best linear threshold, `score ≥ 11.5·L + 5` | 9,861, with TinyCast's formula as written |
| Shown by only one side | 2,681 | 99 (58 by the reference only, 41 by B only) |
| Agreement on shown pairs (both ÷ shown by either) | 0.754 | 0.990 |
| Order among pairs the reference scores apart | 2,239 of 14,375 inverted | 0 inverted, 254 tied by B |
| Queries where a best item of the reference is a best item of frizbee | 1,577 of 1,755 | 1,755 of 1,755 |

The 1-point differences in B have three causes, found by reading frizbee's scorer (`src/smith_waterman/algo/ascii.rs`) and the indices it reports:

- frizbee gives no word start to a letter after a digit (`pass` on `1Password` scores 8, the reference 9) or to the last capital of a run (`edit` on `BBEdit`, 8 against 9).
- Smith-Waterman starts a local alignment for free, so a later query character can take the prefix bonus alone. `ac` on Calculator scores 4 through its `c`, and frizbee reports one index for two query characters. The reference gives 3, below Medium.
- frizbee treats every ASCII character that is not a letter or digit as a separator. TinyCast's list is space, tab, newline, and `- . / ( ) [ ]`.

Query separators. For the 290 queries that contain them, removing every separator before frizbee agreed with the reference on all 305 shown pairs. Normalizing separators to spaces and keeping them agreed on 301.

## Cost

`bench/score-map/examples/cost.rs` timed B alone with `match_list` over 7 haystacks per entry (a name, a generic name, and 5 keywords), on 17 typed queries, as the median of 7 runs. The spread between runs was under 3%, and an earlier run gave 14.8, 47.5, and 464 µs.

| Entries | Haystacks | Time per query |
|---|---|---|
| 300 | 2,100 | 15.0 µs |
| 1,000 | 7,000 | 48.5 µs |
| 10,000 | 70,000 | 465 µs |

That is about 7 ns per haystack. notes/03's "34 to 73 µs on 10k items" counts 10,000 haystacks, which is about 1,400 apps with 7 fields each, not 10,000 apps. The whole ranking path, with the comparator and the rows, is timed by `rank_bench` in [design/ranking.md](../../design/ranking.md).
