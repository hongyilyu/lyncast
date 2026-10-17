use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A count that [`Permit`]s hold parts of, which never passes `max`.
#[derive(Clone)]
pub struct Permits {
    taken: Arc<AtomicU64>,
    max: u64,
}

impl Permits {
    pub fn new(max: u64) -> Permits {
        Permits {
            taken: Arc::default(),
            max,
        }
    }

    /// Whether `n` more would fit, without taking them.
    pub fn room(&self, n: u64) -> bool {
        self.taken
            .load(Ordering::Relaxed)
            .checked_add(n)
            .is_some_and(|taken| taken <= self.max)
    }

    /// A permit for `n` more, or `None` when they would bring the count past its most.
    pub fn take(&self, n: u64) -> Option<Permit> {
        self.taken
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |taken| {
                taken.checked_add(n).filter(|&taken| taken <= self.max)
            })
            .ok()
            .map(|_| Permit {
                taken: Arc::clone(&self.taken),
                n,
            })
    }
}

/// `n` of a [`Permits`] count, given back when dropped.
pub struct Permit {
    taken: Arc<AtomicU64>,
    n: u64,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.taken.fetch_sub(self.n, Ordering::Relaxed);
    }
}
