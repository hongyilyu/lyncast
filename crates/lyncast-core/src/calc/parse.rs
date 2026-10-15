//! The one Pratt parser, and the tree it builds with its echo, badge, and evaluation.

use std::fmt;
use std::ptr;

use super::Env;
use super::catalog::{self, Name};
use super::format::write_literal;
use super::lex::Tok;
use super::ops::{Infix, MUL, NEG, Prefix};
use super::quantity::{EvalError, Num, Quantity};

#[derive(Debug)]
pub(super) enum Expr<'a> {
    /// A literal as typed, with its value.
    Num {
        text: &'a str,
        value: Num,
    },
    Name(Name),
    Prefix(&'static Prefix, Box<Expr<'a>>),
    /// Terms joined left to right by links of one binding power, as in `1 + 2 − 3` or
    /// `6 ÷ 2(1+2)`. A chain is a list, so a long pasted sum does not recurse once per term, and
    /// only groups and prefix operators nest. `rest` is never empty.
    Chain {
        first: Box<Expr<'a>>,
        rest: Vec<(Link, Expr<'a>)>,
    },
}

/// How a term joins the terms before it in a chain.
#[derive(Clone, Copy, Debug)]
pub(super) enum Link {
    Op(&'static Infix),
    /// A group right after an operand, as in `4(2+3)`: a product at `×` strength.
    Juxtaposed,
}

impl Link {
    fn op(self) -> &'static Infix {
        match self {
            Link::Op(op) => op,
            Link::Juxtaposed => &MUL,
        }
    }
}

/// Parentheses and prefix operators nest at most this deep. Deeper input is not typed by hand,
/// and with chains the bound also bounds the recursion, which keeps it off the end of a small
/// thread stack.
const MAX_DEPTH: u8 = 32;

/// The tree for `tokens`, or `None` unless they make exactly one expression.
pub(super) fn parse<'a>(tokens: &[Tok<'a>]) -> Option<Expr<'a>> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth_left: MAX_DEPTH,
    };
    let expr = parser.expr(0)?;
    (parser.pos == tokens.len()).then_some(expr)
}

/// Drops one trailing infix symbol, so `12×34+` still shows `12 × 34`. A trailing word such as
/// `x` does not count.
pub(super) fn strip_trailing_operator(tokens: &mut Vec<Tok>) -> bool {
    let trailing = matches!(
        tokens.last(),
        Some(Tok::Op {
            infix: Some(_),
            symbol: true,
            ..
        })
    );
    if trailing {
        tokens.pop();
    }
    trailing
}

struct Parser<'t, 'a> {
    tokens: &'t [Tok<'a>],
    pos: usize,
    depth_left: u8,
}

impl<'a> Parser<'_, 'a> {
    fn expr(&mut self, min_bp: u8) -> Option<Expr<'a>> {
        let mut lhs = self.operand()?;
        while let Some(link) = self.link(min_bp) {
            if let Link::Op(_) = link {
                self.pos += 1;
            }
            let rhs = self.expr(link.op().bp)?;
            lhs = lhs.joined(link, rhs);
        }
        Some(lhs)
    }

    /// The link at the cursor, if it binds tighter than `min_bp`. A group juxtaposes. A number
    /// never does, so `5 3` stays a search.
    fn link(&self, min_bp: u8) -> Option<Link> {
        let link = match self.tokens.get(self.pos)? {
            Tok::Op {
                infix: Some(op), ..
            } => Link::Op(op),
            Tok::Open => Link::Juxtaposed,
            // How a name binds to the operand before it waits for units, and the compiler
            // points here once `Name` has a value.
            Tok::Word(word) => match catalog::lookup(word)? {},
            _ => return None,
        };
        (link.op().bp > min_bp).then_some(link)
    }

    fn operand(&mut self) -> Option<Expr<'a>> {
        let tok = *self.tokens.get(self.pos)?;
        self.pos += 1;
        match tok {
            Tok::Num(text) => Some(Expr::Num {
                text,
                value: Num::literal(text).ok()?,
            }),
            Tok::Op {
                prefix: Some(op), ..
            } => {
                let arg = self.nested(|p| p.expr(op.bp))?;
                Some(Expr::Prefix(op, Box::new(arg)))
            }
            Tok::Open => {
                let inner = self.nested(|p| p.expr(0))?;
                if !matches!(self.tokens.get(self.pos), Some(Tok::Close)) {
                    return None;
                }
                self.pos += 1;
                Some(inner)
            }
            Tok::Word(word) => Some(Expr::Name(catalog::lookup(word)?)),
            Tok::Op { prefix: None, .. } | Tok::Close | Tok::Radix => None,
        }
    }

    /// Runs `f` one level deeper, or fails past `MAX_DEPTH`.
    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Option<Expr<'a>>) -> Option<Expr<'a>> {
        self.depth_left = self.depth_left.checked_sub(1)?;
        let expr = f(self);
        self.depth_left += 1;
        expr
    }
}

impl<'a> Expr<'a> {
    /// A chain of the link's binding power grows by one term. Links of one binding power
    /// associate left, so `(1+2)+3` may join the inner chain.
    fn joined(self, link: Link, term: Expr<'a>) -> Expr<'a> {
        let grows = self.bp() == link.op().bp;
        match self {
            Expr::Chain { first, mut rest } if grows => {
                rest.push((link, term));
                Expr::Chain { first, rest }
            }
            lhs => Expr::Chain {
                first: Box::new(lhs),
                rest: vec![(link, term)],
            },
        }
    }
}

impl Expr<'_> {
    pub(super) fn eval(&self, env: &Env) -> Result<Quantity, EvalError> {
        match self {
            Expr::Num { value, .. } => Ok(Quantity::scalar(*value)),
            Expr::Name(name) => name.price(env),
            Expr::Prefix(op, arg) => (op.apply)(arg.eval(env)?),
            Expr::Chain { first, rest } => {
                rest.iter().try_fold(first.eval(env)?, |acc, (link, term)| {
                    (link.op().apply)(acc, term.eval(env)?)
                })
            }
        }
    }

    /// A number, negated any number of times. Alone it earns no card.
    pub(super) fn is_bare_number(&self) -> bool {
        match self {
            Expr::Num { .. } => true,
            Expr::Prefix(op, arg) => is_neg(op) && arg.is_bare_number(),
            _ => false,
        }
    }

    /// What the answer is called: the root row's badge, or `Result` for a lone number. A
    /// chain's root is its last link.
    pub(super) fn badge(&self) -> &'static str {
        match self {
            Expr::Prefix(op, _) if !self.is_bare_number() => op.badge,
            Expr::Chain { rest, .. } => rest.last().map_or("Result", |(link, _)| link.op().badge),
            Expr::Name(name) => match *name {},
            Expr::Num { .. } | Expr::Prefix(..) => "Result",
        }
    }

    /// How tightly the root binds, for the echo's parentheses. An operand binds tightest.
    fn bp(&self) -> u8 {
        match self {
            Expr::Chain { rest, .. } => rest.first().map_or(u8::MAX, |(link, _)| link.op().bp),
            Expr::Num { .. } | Expr::Name(_) | Expr::Prefix(..) => u8::MAX,
        }
    }
}

/// The echo: spaced operators, the table's glyphs, and only the parentheses that precedence
/// needs, so `((2))*3` echoes `2 × 3`.
impl fmt::Display for Expr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Num { text, .. } => write_literal(f, text),
            Expr::Name(name) => match *name {},
            Expr::Prefix(op, arg) => {
                f.write_str(op.echo)?;
                // `−(2 + 3)` and `−(−5)` keep their parentheses.
                let wrap =
                    arg.bp() < u8::MAX || matches!(**arg, Expr::Prefix(inner, _) if is_neg(inner));
                wrapped(f, arg, wrap)
            }
            Expr::Chain { first, rest } => {
                let bp = self.bp();
                wrapped(f, first, first.bp() < bp)?;
                for (link, term) in rest {
                    match link {
                        Link::Op(op) => {
                            write!(f, " {} ", op.echo)?;
                            wrapped(f, term, term.bp() <= bp)?;
                        }
                        // A juxtaposed term was typed as a group, so it keeps its parentheses.
                        Link::Juxtaposed => write!(f, "({term})")?,
                    }
                }
                Ok(())
            }
        }
    }
}

fn is_neg(op: &Prefix) -> bool {
    ptr::eq(op, &NEG)
}

fn wrapped(f: &mut fmt::Formatter<'_>, expr: &Expr, wrap: bool) -> fmt::Result {
    if wrap {
        write!(f, "({expr})")
    } else {
        write!(f, "{expr}")
    }
}
