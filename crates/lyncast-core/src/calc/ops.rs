//! The one operator table. Each row owns its spellings, binding power, echo glyph, badge, and
//! arithmetic. A new operator is a new row.

use super::quantity::{EvalError, Quantity};

/// One row of the table.
#[derive(Debug)]
pub(super) struct Operator<F> {
    /// What the user may type. A symbol lexes as one token. A word matches a whole word without
    /// case, and is ASCII.
    pub(super) spellings: &'static [&'static str],
    /// Higher binds tighter. Infix rows associate left.
    pub(super) bp: u8,
    pub(super) echo: &'static str,
    /// What the answer is called when this row is the root.
    pub(super) badge: &'static str,
    pub(super) apply: F,
}

pub(super) type Prefix = Operator<fn(Quantity) -> Result<Quantity, EvalError>>;
pub(super) type Infix = Operator<fn(Quantity, Quantity) -> Result<Quantity, EvalError>>;

pub(super) static ADD: Infix = Operator {
    spellings: &["+"],
    bp: 10,
    echo: "+",
    badge: "Sum",
    apply: Quantity::checked_add,
};

pub(super) static SUB: Infix = Operator {
    spellings: &["-", "−"],
    bp: 10,
    echo: "−",
    badge: "Difference",
    apply: Quantity::checked_sub,
};

/// `x` is a word, so it means `×` only after an operand, and `xterm` and `12 x` stay searches.
pub(super) static MUL: Infix = Operator {
    spellings: &["*", "×", "·", "x"],
    bp: 20,
    echo: "×",
    badge: "Product",
    apply: Quantity::checked_mul,
};

pub(super) static DIV: Infix = Operator {
    spellings: &["/", "÷"],
    bp: 20,
    echo: "÷",
    badge: "Quotient",
    apply: Quantity::checked_div,
};

pub(super) static NEG: Prefix = Operator {
    spellings: &["-", "−"],
    bp: 30,
    echo: "−",
    badge: "Negation",
    apply: Quantity::checked_neg,
};

pub(super) static INFIX: [&Infix; 4] = [&ADD, &SUB, &MUL, &DIV];
pub(super) static PREFIX: [&Prefix; 1] = [&NEG];

#[cfg(test)]
mod tests {
    use super::{INFIX, PREFIX};

    /// The lexer tells a word spelling from a symbol by its first byte.
    #[test]
    fn word_spellings_are_ascii() {
        let spellings = INFIX
            .iter()
            .flat_map(|op| op.spellings)
            .chain(PREFIX.iter().flat_map(|op| op.spellings));
        for spelling in spellings {
            let word = spelling.starts_with(char::is_alphabetic);
            assert!(!word || spelling.is_ascii(), "{spelling}");
        }
    }
}
