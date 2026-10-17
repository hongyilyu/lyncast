//! The cost of one frizbee pass with TinyCast's points, on 300, 1,000, and 10,000 entries of 7
//! haystacks each. Run it with
//! `cargo run --release --manifest-path bench/score-map/Cargo.toml --target-dir target/bench-score-map --example cost`.

use frizbee::{Config, Matcher, Scoring};
use std::hint::black_box;
use std::time::Instant;

const NAMES: &str = include_str!("../src/names.txt");
const GENERIC: [&str; 8] = [
    "Web Browser",
    "Text Editor",
    "File Manager",
    "Terminal Emulator",
    "Image Viewer",
    "Media Player",
    "Office Suite",
    "Email Client",
];
const KW: [&str; 10] = [
    "internet", "www", "edit", "code", "files", "shell", "photo", "music", "document", "mail",
];

fn main() {
    let names: Vec<&str> = NAMES.lines().filter(|l| !l.is_empty()).collect();
    let cfg = Config::default().scoring(Scoring {
        match_score: 2,
        mismatch_penalty: 4,
        gap_open_penalty: 1,
        gap_extend_penalty: 0,
        prefix_bonus: 2,
        capitalization_bonus: 1,
        matching_case_bonus: 0,
        exact_match_bonus: 0,
        delimiter_bonus: 1,
    });
    let queries = [
        "f", "fi", "fir", "fire", "firef", "firefo", "c", "co", "cod", "code", "term", "vsc", "lo",
        "clip", "set", "gimp", "xyzq",
    ];
    for n in [300usize, 1000, 10_000] {
        // haystacks: per entry name, generic, and 5 keywords = 7 haystacks
        let mut hay: Vec<String> = Vec::new();
        for i in 0..n {
            let name = names[i % names.len()];
            hay.push(if i < names.len() {
                name.to_string()
            } else {
                format!("{name} {}", i / names.len())
            });
            hay.push(GENERIC[i % 8].to_string());
            for k in 0..5 {
                hay.push(KW[(i + k) % 10].to_string());
            }
        }
        let mut samples = Vec::new();
        for _ in 0..7 {
            let t = Instant::now();
            let mut total = 0usize;
            for _ in 0..50 {
                for q in &queries {
                    let mut m = Matcher::new(*q, &cfg);
                    total += black_box(m.match_list(&hay)).len();
                }
            }
            let per = t.elapsed().as_secs_f64() * 1e6 / (50.0 * queries.len() as f64);
            samples.push(per);
            black_box(total);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "entries {n:>6} haystacks {:>6}: {:.1} us per query (median of 7, range {:.1}-{:.1})",
            hay.len(),
            samples[3],
            samples[0],
            samples[6]
        );
    }
}
