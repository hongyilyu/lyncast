//! How do frizbee 0.13.0 scores map onto TinyCast's point scorer and its Medium threshold?
//! The reference scorer is written from TinyCast's launcher doc (its scorer section), not from
//! TinyCast code. docs/research/notes/10 records the run.

use frizbee::{Config, Matcher, Scoring};
use std::collections::BTreeMap;

const NAMES: &str = include_str!("names.txt");

fn is_sep(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '-' | '.' | '/' | '(' | ')' | '[' | ']'
    )
}

fn word_starts(t: &[char]) -> Vec<bool> {
    let ascii = t.iter().all(|c| c.is_ascii());
    (0..t.len())
        .map(|i| {
            let c = t[i];
            if is_sep(c) {
                return false;
            }
            if i == 0 || is_sep(t[i - 1]) {
                return true;
            }
            if !ascii {
                return false;
            }
            let p = t[i - 1];
            (c.is_ascii_uppercase() && p.is_ascii_lowercase())
                || (c.is_ascii_alphabetic() && p.is_ascii_digit())
                || (c.is_ascii_uppercase()
                    && p.is_ascii_uppercase()
                    && t.get(i + 1).is_some_and(|n| n.is_ascii_lowercase()))
        })
        .collect()
}

/// TinyCast's scorer from the doc: best alignment, (score, L). None if no alignment.
/// L is the query length less skipped separators. Ties prefer fewer skips.
fn tiny(query: &str, text: &str) -> Option<(i32, usize)> {
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let low: Vec<char> = t.iter().map(|c| c.to_ascii_lowercase()).collect();
    let ws = word_starts(&t);
    let n = t.len();
    const NEG: (i32, i32) = (i32::MIN / 4, 0);
    // cur[j] = best (score, -skips) with the last match at j
    let mut cur = vec![NEG; n];
    let mut none: Option<i32> = Some(0); // -skips if nothing matched yet
    for &qc in &q {
        let sep = is_sep(qc);
        let mut next = vec![NEG; n];
        if sep {
            for j in 0..n {
                if cur[j].0 > NEG.0 {
                    next[j] = (cur[j].0, cur[j].1 - 1);
                }
            }
        }
        let mut far = NEG;
        for j in 0..n {
            if j >= 2 && cur[j - 2] > far {
                far = cur[j - 2];
            }
            let tc = t[j];
            let base = if sep {
                if !is_sep(tc) {
                    continue;
                }
                if tc == qc { 2 } else { 1 }
            } else {
                if low[j] != qc {
                    continue;
                }
                if j == 0 {
                    4
                } else if ws[j] {
                    3
                } else {
                    2
                }
            };
            let mut v = NEG;
            if let Some(s) = none {
                v = v.max((base, s));
            }
            if j >= 1 && cur[j - 1].0 > NEG.0 {
                v = v.max((cur[j - 1].0 + base, cur[j - 1].1));
            }
            if far.0 > NEG.0 {
                v = v.max((far.0 + base - 1, far.1));
            }
            next[j] = next[j].max(v);
        }
        none = if sep { none.map(|s| s - 1) } else { None };
        cur = next;
    }
    let best = cur.into_iter().max()?;
    if best.0 <= NEG.0 {
        return None;
    }
    Some((best.0, (q.len() as i32 + best.1) as usize))
}

fn medium(score: f64, l: usize) -> bool {
    score >= 1.5 * (l as f64 - 2.0) + 4.0
}

fn tiny_weights() -> Scoring {
    Scoring {
        match_score: 2,
        mismatch_penalty: 4,
        gap_open_penalty: 1,
        gap_extend_penalty: 0,
        prefix_bonus: 2,
        capitalization_bonus: 1,
        matching_case_bonus: 0,
        exact_match_bonus: 0,
        delimiter_bonus: 1,
    }
}

fn friz(cfg: &Config, query: &str, text: &str) -> Option<u16> {
    let mut m = Matcher::new(query, cfg);
    m.match_one(text, 0).map(|m| m.score)
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Queries per name: prefixes, initials, mid-word runs, random subsequences.
fn queries(names: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut plain = std::collections::BTreeSet::new();
    let mut multi = std::collections::BTreeSet::new();
    let mut rng = Lcg(42);
    for name in names {
        let low: String = name.to_lowercase();
        let chars: Vec<char> = low.chars().collect();
        for len in 1..=chars.len().min(10) {
            let p: String = chars[..len].iter().collect();
            if p.chars().any(is_sep) {
                if !p.ends_with(is_sep) {
                    multi.insert(p);
                }
            } else {
                plain.insert(p);
            }
        }
        let words: Vec<&str> = low.split(is_sep).filter(|w| !w.is_empty()).collect();
        if words.len() >= 2 {
            plain.insert(
                words
                    .iter()
                    .filter_map(|w| w.chars().next())
                    .collect::<String>(),
            );
            multi.insert(words.join(" "));
            multi.insert(format!(
                "{} {}",
                words[0],
                words[1].chars().take(2).collect::<String>()
            ));
        }
        let letters: Vec<char> = chars.iter().copied().filter(|c| !is_sep(*c)).collect();
        for w in &words {
            let wc: Vec<char> = w.chars().collect();
            for start in 1..wc.len() {
                for len in 2..=5 {
                    if start + len <= wc.len() {
                        plain.insert(wc[start..start + len].iter().collect::<String>());
                    }
                }
            }
        }
        for _ in 0..4 {
            if letters.len() < 3 {
                break;
            }
            let k = 2 + rng.below(letters.len().min(6) - 1);
            let mut idx: Vec<usize> = (0..letters.len()).collect();
            for i in (1..idx.len()).rev() {
                idx.swap(i, rng.below(i + 1));
            }
            let mut pick = idx[..k.min(letters.len())].to_vec();
            pick.sort();
            plain.insert(pick.iter().map(|&i| letters[i]).collect::<String>());
        }
    }
    (plain.into_iter().collect(), multi.into_iter().collect())
}

#[derive(Default)]
struct Confusion {
    both: usize,
    ref_only: usize,
    cand_only: usize,
    neither: usize,
}
impl Confusion {
    fn add(&mut self, r: bool, c: bool) {
        match (r, c) {
            (true, true) => self.both += 1,
            (true, false) => self.ref_only += 1,
            (false, true) => self.cand_only += 1,
            (false, false) => self.neither += 1,
        }
    }
    fn agree(&self) -> f64 {
        let shown = self.both + self.ref_only + self.cand_only;
        self.both as f64 / shown.max(1) as f64
    }
}

fn main() {
    let names: Vec<&str> = NAMES.lines().filter(|l| !l.is_empty()).collect();
    let (plain, multi) = queries(&names);
    println!(
        "names {} plain queries {} multiword queries {}",
        names.len(),
        plain.len(),
        multi.len()
    );

    let cfg_a = Config::default();
    let cfg_b = Config::default().scoring(tiny_weights());

    // One aligned pair: the query's index in `plain`, the name's index, the reference's
    // (score, L), exactness, and A's and B's scores.
    struct Pair {
        q: usize,
        n: usize,
        r: Option<(i32, usize)>,
        exact: bool,
        a: Option<u16>,
        b: Option<u16>,
    }
    let mut pairs = Vec::new();
    for (qi, q) in plain.iter().enumerate() {
        let mut ma = Matcher::new(q.as_str(), &cfg_a);
        let mut mb = Matcher::new(q.as_str(), &cfg_b);
        for (ni, name) in names.iter().enumerate() {
            let r = tiny(q, name);
            let a = ma.match_one(name, 0).map(|m| m.score);
            let b = mb.match_one(name, 0).map(|m| m.score);
            if r.is_none() && a.is_none() && b.is_none() {
                continue;
            }
            pairs.push(Pair {
                q: qi,
                n: ni,
                r,
                exact: name.to_lowercase() == *q,
                a,
                b,
            });
        }
    }
    println!("aligned pairs {}", pairs.len());
    let aligned_mismatch = pairs
        .iter()
        .filter(|p| p.r.is_some() != p.b.is_some())
        .count();
    println!("pairs where frizbee alignment existence differs from reference: {aligned_mismatch}");

    // 1. Score equality ref vs B
    let mut diff: BTreeMap<i32, usize> = BTreeMap::new();
    for p in &pairs {
        if let (Some((s, _)), Some(b)) = (p.r, p.b) {
            *diff.entry(b as i32 - s).or_default() += 1;
        }
    }
    println!("\nB score minus reference score (count):");
    for (d, c) in &diff {
        println!("  {d:+}: {c}");
    }

    // 2. Medium decisions
    let shows_ref = |p: &Pair| p.exact || p.r.is_some_and(|(s, l)| medium(s as f64, l));
    let mut cb = Confusion::default();
    for p in &pairs {
        cb.add(
            shows_ref(p),
            p.exact
                || p.b
                    .is_some_and(|b| medium(b as f64, plain[p.q].chars().count())),
        );
    }
    println!(
        "\nMedium, B with TinyCast's formula: both {} ref-only {} B-only {} (jaccard {:.4})",
        cb.both,
        cb.ref_only,
        cb.cand_only,
        cb.agree()
    );
    // A: fit threshold a*L+b on default frizbee scores
    let mut best = (0.0, 0.0, 0.0, Confusion::default());
    for a10 in 0..=200 {
        let a = a10 as f64 * 0.1;
        for b in -40..=40 {
            let b = b as f64;
            let mut c = Confusion::default();
            for p in &pairs {
                let l = plain[p.q].chars().count() as f64;
                c.add(
                    shows_ref(p),
                    p.exact || p.a.is_some_and(|s| s as f64 >= a * l + b),
                );
            }
            if c.agree() > best.2 {
                best = (a, b, c.agree(), c);
            }
        }
    }
    let c = &best.3;
    println!(
        "Medium, A (default weights) best fit score >= {:.1}*L {:+}: both {} ref-only {} A-only {} (jaccard {:.4})",
        best.0, best.1, c.both, c.ref_only, c.cand_only, best.2
    );

    // 3. Ordering among pairs both show, per query: pairwise concordance on score
    for (label, pick) in [("B", true), ("A", false)] {
        let mut by_q: BTreeMap<usize, Vec<(i32, i32)>> = BTreeMap::new();
        for p in &pairs {
            let cand = if pick { p.b } else { p.a };
            if let (Some((s, l)), Some(c)) = (p.r, cand)
                && medium(s as f64, l)
            {
                by_q.entry(p.q).or_default().push((s, c as i32));
            }
        }
        let (mut conc, mut disc, mut tie_c, mut top1, mut top1n) =
            (0usize, 0usize, 0usize, 0usize, 0usize);
        for v in by_q.values() {
            for i in 0..v.len() {
                for j in i + 1..v.len() {
                    let (r1, c1) = v[i];
                    let (r2, c2) = v[j];
                    if r1 == r2 {
                        continue;
                    }
                    match (r1 > r2, c1.cmp(&c2)) {
                        (_, std::cmp::Ordering::Equal) => tie_c += 1,
                        (true, std::cmp::Ordering::Greater) | (false, std::cmp::Ordering::Less) => {
                            conc += 1
                        }
                        _ => disc += 1,
                    }
                }
            }
            if v.len() >= 2 {
                top1n += 1;
                let rmax = v.iter().map(|x| x.0).max().unwrap();
                let cmax = v.iter().map(|x| x.1).max().unwrap();
                let ref_top: Vec<_> = v.iter().filter(|x| x.0 == rmax).collect();
                if ref_top.iter().any(|x| x.1 == cmax) {
                    top1 += 1;
                }
            }
        }
        println!(
            "Order {label} vs reference over ref-separated pairs: concordant {conc} discordant {disc} candidate-tied {tie_c}; a ref-best item is candidate-best in {top1}/{top1n} queries"
        );
    }

    // 4. Doc examples
    println!("\nDoc examples (query, text): ref (score,L) medium | B | A");
    for (q, t) in [
        ("code", "Xcode"),
        ("pec", "Accessibility Inspector"),
        ("pec", "Previous Track"),
        ("vsc", "Visual Studio Code"),
        ("chrome", "Google Chrome"),
        ("stack", "OrbStack"),
        ("ap", "App Store"),
        ("ap", "AirPort Utility"),
        ("edit", "BBEdit"),
        ("pass", "1Password"),
        ("firef", "Firefox"),
        ("firef", "Firefox Web Browser"),
        ("ff", "Firefox"),
        ("term", "Terminal"),
        ("term", "GNOME Terminal"),
        ("clip", "Clipboard History"),
        ("off", "Power Off"),
        ("lo", "Log Out"),
        ("lo", "Lock Screen"),
        ("lo", "LibreOffice"),
    ] {
        let r = tiny(q, t);
        println!(
            "  {q:>6} {t:<26} {:?} {} | {:?} {} | {:?}",
            r,
            r.is_some_and(|(s, l)| medium(s as f64, l)),
            friz(&cfg_b, q, t),
            friz(&cfg_b, q, t).is_some_and(|b| medium(b as f64, q.len())),
            friz(&cfg_a, q, t),
        );
    }

    // 5. The first 25 Medium disagreements for B, in query order
    println!("\nSample B disagreements on Medium (query, name, ref, B):");
    let mut shown = 0;
    for p in &pairs {
        let r = shows_ref(p);
        let b = p.exact
            || p.b
                .is_some_and(|b| medium(b as f64, plain[p.q].chars().count()));
        if r != b && shown < 25 {
            println!("  {:>8} {:<32} {:?} {:?}", plain[p.q], names[p.n], p.r, p.b);
            shown += 1;
        }
    }

    // 6. Multiword queries: S1 keep separators (normalized to space), S2 strip them
    let norm = |s: &str| {
        s.chars()
            .map(|c| if is_sep(c) { ' ' } else { c })
            .collect::<String>()
    };
    let mut s1 = Confusion::default();
    let mut s2 = Confusion::default();
    for q in &multi {
        let stripped: String = q.chars().filter(|c| !is_sep(*c)).collect();
        let q1 = norm(q);
        let mut m1 = Matcher::new(q1.as_str(), &cfg_b);
        let mut m2 = Matcher::new(stripped.as_str(), &cfg_b);
        for name in &names {
            let exact = name.to_lowercase() == *q;
            let r = exact || tiny(q, name).is_some_and(|(s, l)| medium(s as f64, l));
            let c1 = exact
                || m1
                    .match_one(norm(name), 0)
                    .is_some_and(|m| medium(m.score as f64, q1.chars().count()));
            let c2 = exact
                || m2
                    .match_one(name, 0)
                    .is_some_and(|m| medium(m.score as f64, stripped.chars().count()));
            s1.add(r, c1);
            s2.add(r, c2);
        }
    }
    println!(
        "\nMultiword ({}): S1 keep seps: both {} ref-only {} cand-only {} (jaccard {:.4}); S2 strip seps: both {} ref-only {} cand-only {} (jaccard {:.4})",
        multi.len(),
        s1.both,
        s1.ref_only,
        s1.cand_only,
        s1.agree(),
        s2.both,
        s2.ref_only,
        s2.cand_only,
        s2.agree()
    );
}
