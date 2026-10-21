//! How a block spells its steps and what each one prints, for the session runner and the
//! palette's Tier 3 runner alike.

use super::doc::Block;

/// One `$ ` line of a block and the lines after it, up to the next `$ ` line.
pub struct Step<'b> {
    /// The line of the doc it is on.
    pub line: usize,
    pub command: &'b str,
    pub after: &'b [String],
}

/// The block's steps in order. The first line is a step, whatever it starts with.
pub fn steps(block: &Block) -> Vec<Step<'_>> {
    let mut steps = Vec::new();
    let mut rest = block.lines.as_slice();
    while let [command, after @ ..] = rest {
        let printed = after
            .iter()
            .take_while(|line| !line.starts_with("$ "))
            .count();
        steps.push(Step {
            line: block.line + 1 + block.lines.len() - rest.len(),
            command,
            after: &after[..printed],
        });
        rest = &after[printed..];
    }
    steps
}

/// What a command printed, each line with its newline, and the lines it added to the recorder.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Printed {
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
    pub record: String,
}

impl Printed {
    /// The lines after a step.
    pub fn expected(lines: &[String]) -> Printed {
        let mut printed = Printed::default();
        for line in lines {
            if let Some(line) = line.strip_prefix("stderr: ") {
                printed.stderr += &format!("{line}\n");
            } else if let Some(code) = line.strip_prefix("exit: ") {
                printed.exit = code.parse().expect("exit: <n>");
            } else if let Some(argv) = line.strip_prefix("record: ") {
                printed.record += &format!("{argv}\n");
            } else {
                printed.stdout += &format!("{line}\n");
            }
        }
        printed
    }
}
