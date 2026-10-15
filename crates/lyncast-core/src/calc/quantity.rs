//! The value every expression evaluates to: a number and its dimension.
//!
//! A scalar is a quantity whose dimension has no exponents. The MVP has no base dimensions, so
//! `Dim` holds nothing yet.

/// Why an expression has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EvalError {
    DivideByZero,
    /// Past `i128`. A step along the way counts too, even when the reduced answer would fit.
    Overflow,
    /// Quantities of different dimensions added together.
    Mismatch,
}

/// An exact fraction in lowest terms with a positive denominator. Every MVP operator keeps a
/// fraction exact, so nothing rounds before formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Num {
    pub(super) n: i128,
    pub(super) d: i128,
}

/// Every caller passes a positive denominator as `b`, so the gcd fits back in an `i128`.
fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a as i128
}

impl Num {
    /// `n / d` in lowest terms, for `d > 0`.
    fn reduced(n: i128, d: i128) -> Num {
        let g = gcd(n, d);
        Num { n: n / g, d: d / g }
    }

    /// The value of a literal: ASCII digits with at most one point. Trailing zeros after the
    /// point add nothing, so they do not count against `i128`.
    pub(super) fn literal(text: &str) -> Result<Num, EvalError> {
        let (int, frac) = text.split_once('.').unwrap_or((text, ""));
        let frac = frac.trim_end_matches('0');
        let mut n: i128 = 0;
        for digit in int.bytes().chain(frac.bytes()) {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add(i128::from(digit - b'0')))
                .ok_or(EvalError::Overflow)?;
        }
        let d = u32::try_from(frac.len())
            .ok()
            .and_then(|places| 10i128.checked_pow(places))
            .ok_or(EvalError::Overflow)?;
        Ok(Num::reduced(n, d))
    }

    /// Over the least common denominator, which overflows later than `d1 * d2` would.
    fn checked_add(self, o: Num) -> Result<Num, EvalError> {
        let g = gcd(self.d, o.d);
        let (a, b) = (self.d / g, o.d / g);
        let n = self
            .n
            .checked_mul(b)
            .zip(o.n.checked_mul(a))
            .and_then(|(x, y)| x.checked_add(y));
        let d = self.d.checked_mul(b);
        Ok(Num::reduced(
            n.ok_or(EvalError::Overflow)?,
            d.ok_or(EvalError::Overflow)?,
        ))
    }

    /// Cancels across first, so the products stay as small as the answer allows. A zero factor
    /// cancels its partner's denominator, so the product is `0/1`.
    fn checked_mul(self, o: Num) -> Result<Num, EvalError> {
        let (g1, g2) = (gcd(self.n, o.d), gcd(o.n, self.d));
        let n = (self.n / g1).checked_mul(o.n / g2);
        let d = (self.d / g2).checked_mul(o.d / g1);
        Ok(Num {
            n: n.ok_or(EvalError::Overflow)?,
            d: d.ok_or(EvalError::Overflow)?,
        })
    }

    /// Multiplies by the reciprocal, with the divisor's sign moved to the top.
    fn checked_div(self, o: Num) -> Result<Num, EvalError> {
        let recip = match o.n {
            0 => return Err(EvalError::DivideByZero),
            n if n < 0 => Num {
                n: -o.d,
                d: n.checked_neg().ok_or(EvalError::Overflow)?,
            },
            n => Num { n: o.d, d: n },
        };
        self.checked_mul(recip)
    }

    fn checked_neg(self) -> Result<Num, EvalError> {
        Ok(Num {
            n: self.n.checked_neg().ok_or(EvalError::Overflow)?,
            d: self.d,
        })
    }
}

/// How many base dimensions exist. Units add them.
const BASES: usize = 0;

/// Exponents of the base dimensions. Products add them, and quotients subtract them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dim([i8; BASES]);

impl Dim {
    const NONE: Dim = Dim([0; BASES]);

    fn combine(self, o: Dim, sign: i8) -> Result<Dim, EvalError> {
        let mut d = self;
        for (a, b) in d.0.iter_mut().zip(o.0) {
            *a = b
                .checked_mul(sign)
                .and_then(|b| a.checked_add(b))
                .ok_or(EvalError::Overflow)?;
        }
        Ok(d)
    }
}

/// A number and its dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Quantity {
    num: Num,
    dim: Dim,
}

impl Quantity {
    pub(super) fn scalar(num: Num) -> Quantity {
        Quantity {
            num,
            dim: Dim::NONE,
        }
    }

    /// The number, when the quantity has no dimension. Units will name the others.
    pub(super) fn into_scalar(self) -> Option<Num> {
        (self.dim == Dim::NONE).then_some(self.num)
    }

    pub(super) fn checked_add(self, o: Quantity) -> Result<Quantity, EvalError> {
        if self.dim != o.dim {
            return Err(EvalError::Mismatch);
        }
        Ok(Quantity {
            num: self.num.checked_add(o.num)?,
            dim: self.dim,
        })
    }

    pub(super) fn checked_sub(self, o: Quantity) -> Result<Quantity, EvalError> {
        self.checked_add(o.checked_neg()?)
    }

    pub(super) fn checked_mul(self, o: Quantity) -> Result<Quantity, EvalError> {
        Ok(Quantity {
            num: self.num.checked_mul(o.num)?,
            dim: self.dim.combine(o.dim, 1)?,
        })
    }

    pub(super) fn checked_div(self, o: Quantity) -> Result<Quantity, EvalError> {
        Ok(Quantity {
            num: self.num.checked_div(o.num)?,
            dim: self.dim.combine(o.dim, -1)?,
        })
    }

    pub(super) fn checked_neg(self) -> Result<Quantity, EvalError> {
        Ok(Quantity {
            num: self.num.checked_neg()?,
            dim: self.dim,
        })
    }
}
