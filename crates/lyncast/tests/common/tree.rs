//! A doc's file blocks written as a tree, and its env block read as variables.

use std::ffi::CString;
use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use super::doc;

/// Writes every `desktop <path>`, `executable <path>`, and `file <path>` block under `tree`, from
/// scratch, makes each `symlink <path> -> <target>` block a link to `target` as written, and makes
/// each `fifo <path>` block a FIFO. A `desktop` block may give `mode=<octal>` after its path.
pub fn materialize(blocks: &[doc::Block], tree: &Path) {
    match fs::remove_dir_all(tree) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{tree:?}: {error}"),
        _ => {}
    }
    fs::create_dir_all(tree).unwrap();
    let of = |kind: &'static str| blocks.iter().filter(move |block| block.kind == kind);
    for kind in ["desktop", "executable", "file"] {
        for block in of(kind) {
            let (relative, mode) = match block.args.as_slice() {
                [relative] => (relative, (kind == "executable").then_some(0o755)),
                [relative, mode] if kind == "desktop" => {
                    let mode = mode
                        .strip_prefix("mode=")
                        .map(|m| u32::from_str_radix(m, 8));
                    let Some(Ok(mode)) = mode else {
                        panic!("line {}: not mode=<octal>", block.line);
                    };
                    (relative, Some(mode))
                }
                _ => panic!("line {}: a {kind} block names one path", block.line),
            };
            let path = tree.join(relative);
            fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
            fs::write(&path, raw_bytes(&format!("{}\n", block.lines.join("\n")))).unwrap();
            if let Some(mode) = mode {
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            }
        }
    }
    for block in of("symlink") {
        let [relative, arrow, target] = block.args.as_slice() else {
            panic!(
                "line {}: a symlink block reads <path> -> <target>",
                block.line
            );
        };
        assert!(
            arrow == "->" && block.lines.is_empty(),
            "line {}: a symlink block reads <path> -> <target> and holds no lines",
            block.line
        );
        let path = tree.join(relative);
        fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
        symlink(target, &path).unwrap();
    }
    for block in of("fifo") {
        let [relative] = block.args.as_slice() else {
            panic!("line {}: a fifo block names one path", block.line);
        };
        assert!(
            block.lines.is_empty(),
            "line {}: a fifo block holds no lines",
            block.line
        );
        let path = tree.join(relative);
        fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
        mkfifo(&path, 0o644);
    }
}

/// Makes a FIFO at `path` with `mode`.
pub fn mkfifo(path: &Path, mode: libc::mode_t) {
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `c_path` is a NUL-terminated string that lives through the call.
    let made = unsafe { libc::mkfifo(c_path.as_ptr(), mode) };
    assert_eq!(made, 0, "mkfifo {path:?}: {}", io::Error::last_os_error());
}

/// The text's bytes, with each exact `\x{HH}` written as the byte `HH`, so a desktop file can
/// hold a byte that is not UTF-8.
pub fn raw_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\\x{") {
        out.extend_from_slice(&rest.as_bytes()[..at]);
        let after = &rest[at + 3..];
        match after.get(..3).and_then(|hex| {
            let digits = hex.strip_suffix('}')?;
            digits
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
                .then_some(())?;
            u8::from_str_radix(digits, 16).ok()
        }) {
            Some(byte) => {
                out.push(byte);
                rest = &after[3..];
            }
            None => {
                out.extend_from_slice(b"\\x{");
                rest = after;
            }
        }
    }
    out.extend_from_slice(rest.as_bytes());
    out
}

/// The one `env` block's `NAME=value` lines, with each `$NAME` in `paths` replaced.
pub fn environment(blocks: &[doc::Block], paths: &[(&str, &Path)]) -> Vec<(String, String)> {
    let envs: Vec<&doc::Block> = blocks.iter().filter(|block| block.kind == "env").collect();
    let [block] = envs.as_slice() else {
        panic!("the doc needs one env block, not {}", envs.len());
    };
    block
        .lines
        .iter()
        .map(|line| {
            let (name, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("not NAME=value: {line:?}"));
            (name.to_owned(), substitute(value, paths))
        })
        .collect()
}

/// `text` with each `$NAME` in `paths` replaced by its path.
pub fn substitute(text: &str, paths: &[(&str, &Path)]) -> String {
    paths.iter().fold(text.to_owned(), |text, (name, path)| {
        text.replace(name, path.to_str().expect("a UTF-8 test path"))
    })
}
