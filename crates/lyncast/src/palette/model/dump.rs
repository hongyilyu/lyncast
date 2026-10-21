use std::fmt::Write;

use lyncast_core::Row;

use super::{
    Cap, Footer, Item, Line, MENU_PLACEHOLDER, Model, NO_ACTIONS, Overlay, card_halves, kind_label,
    question,
};

impl Model {
    /// The text of palette.md's `palette` blocks: the field, the lines with the selected row
    /// marked, the clipboard screen's `preview`, which the view gives, and its information, the
    /// overlay, and the footer.
    pub fn dump(&self, preview: Option<&str>) -> String {
        let mut out = String::new();
        let field = if self.field.shows_placeholder() {
            format!("({})", self.placeholder())
        } else {
            format!("{:?}", self.field.text())
        };
        let _ = match self.kinds() {
            None => writeln!(out, "root {field}"),
            Some(kinds) => writeln!(out, "clipboard {field} {}", kinds.label()),
        };
        for line in &self.lines {
            let _ = match *line {
                Line::Header(heading) => writeln!(out, "== {}", heading.label()),
                Line::Row(index) => {
                    let mark = if self.is_selected(index) { ">" } else { " " };
                    match self.item(index) {
                        Item::Row(row) => writeln!(out, "{mark} {}", row_text(row)),
                        Item::Clip(clip) => {
                            writeln!(out, "{mark} {}: {}", clip.content.spelling(), clip.title)
                        }
                    }
                }
            };
        }
        if self.lines.is_empty() {
            let _ = writeln!(out, "{}", self.empty_text());
        }
        if let Some(preview) = preview {
            let _ = writeln!(out, "preview: {preview}");
        }
        if let Some(clip) = self.selected_clip() {
            let info: Vec<String> = clip
                .info()
                .into_iter()
                .map(|(label, value)| format!("{label} {value}"))
                .collect();
            let _ = writeln!(out, "info: {}", info.join(" · "));
        }
        match &self.overlay {
            Some(Overlay::Confirm { title, .. }) => {
                let _ = writeln!(out, "confirm {}", question(title));
            }
            Some(Overlay::Menu(menu)) => {
                if menu.field.shows_placeholder() {
                    let _ = writeln!(out, "menu ({MENU_PLACEHOLDER})");
                } else {
                    let _ = writeln!(out, "menu {:?}", menu.field.text());
                }
                for (at, (chord, item)) in menu.shown().enumerate() {
                    let mark = if at == menu.selected_shown { ">" } else { " " };
                    let _ = match chord {
                        Some(chord) => {
                            writeln!(
                                out,
                                "{mark} {}  ({})",
                                item.label(),
                                Cap::of(*chord).spelling()
                            )
                        }
                        None => writeln!(out, "{mark} {}", item.label()),
                    };
                }
                if menu.shown().next().is_none() {
                    let _ = writeln!(out, "{NO_ACTIONS}");
                }
            }
            None => {}
        }
        let _ = match self.footer() {
            Some(Footer::Pending) => writeln!(out, "footer: pending"),
            Some(Footer::Closing) => writeln!(out, "footer: closing"),
            Some(Footer::Failed(reason)) => writeln!(out, "footer: failed: {reason}"),
            Some(footer) => {
                let hints: Vec<String> = (footer.hints().iter())
                    .map(|(label, cap)| format!("{label} ({})", cap.spelling()))
                    .collect();
                writeln!(out, "footer: {}", hints.join(" · "))
            }
            None => Ok(()),
        };
        out
    }
}

impl Cap {
    /// How a `palette` block spells the key.
    pub fn spelling(self) -> &'static str {
        match self {
            Cap::Enter => "enter",
            Cap::Escape => "escape",
            Cap::CtrlK => "ctrl-k",
            Cap::CtrlEnter => "ctrl-enter",
        }
    }
}

fn row_text(row: &Row) -> String {
    if let Some(card) = &row.card {
        let [(expression, source), (display, target)] = card_halves(card);
        let badges: Vec<&str> = [Some(expression), source, target]
            .into_iter()
            .flatten()
            .collect();
        return format!("= {display}  ({})", badges.join(" · "));
    }
    match kind_label(row.subject.key().kind()) {
        Some(kind) => format!("{}  ({kind})", row.title),
        None => row.title.clone(),
    }
}
