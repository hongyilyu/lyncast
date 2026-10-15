//! lyncast-core stays pure: no UI, Wayland, clipboard, storage, or niri crate
//! may appear anywhere in its dependency graph (docs/design/architecture.md).

use std::path::Path;
use std::process::Command;

fn banned_deps(manifest: &Path) -> Vec<String> {
    let out = Command::new(env!("CARGO"))
        .args(["tree", "--locked", "-p", "lyncast-core", "--all-features"])
        .args(["--target", "all", "--prefix", "none", "--format", "{p}"])
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "cargo tree failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("cargo tree prints UTF-8");
    let mut banned: Vec<String> = stdout
        .lines()
        .map(|line| line.split_once(' ').map_or(line, |(name, _)| name))
        .filter(|name| is_banned(name))
        .map(str::to_owned)
        .collect();
    banned.sort();
    banned.dedup();
    banned
}

fn is_banned(name: &str) -> bool {
    name == "gpui"
        || name.starts_with("gpui-")
        || name.starts_with("gpui_")
        || name.starts_with("wayland-")
        || matches!(name, "wl-clipboard-rs" | "rusqlite" | "niri-ipc")
}

#[test]
fn lyncast_core_has_no_banned_dependencies() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    assert_eq!(banned_deps(&workspace), Vec::<String>::new());
}

#[test]
fn banned_crates_are_found_behind_every_kind_of_edge() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/impure/Cargo.toml");
    assert_eq!(
        banned_deps(&fixture),
        ["gpui-pre", "niri-ipc", "rusqlite", "wayland-client"]
    );
}
