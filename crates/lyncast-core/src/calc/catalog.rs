//! Names that evaluate to a quantity: units, currencies, and constants. The parser asks here
//! about any word the operator table does not spell. Evaluation prices the name with `Env`, so
//! parsing never reads `Env`.

use super::Env;
use super::quantity::{EvalError, Quantity};

/// A name the catalog knows. The MVP knows none, so this type has no values, and the compiler
/// will point at every place that handles a name once units add the first one.
#[derive(Clone, Copy, Debug)]
pub(super) enum Name {}

impl Name {
    /// The name's value. A currency will read its rate from `env` here.
    pub(super) fn price(self, _env: &Env) -> Result<Quantity, EvalError> {
        match self {}
    }
}

/// The name that `word` spells, if any.
pub(super) fn lookup(_word: &str) -> Option<Name> {
    None
}
