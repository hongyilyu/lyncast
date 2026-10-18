//! glibc reads the zone file that `TZ` names, or `/etc/localtime`, at the first `localtime_r`,
//! holding its zone lock, and never again for `localtime_r` (glibc 2.34 `time/tzset.c:367-374`
//! and `567-577`, `time/localtime.c:28-41`, read in source). That read can wait forever, on a
//! FIFO for one, so the palette makes it once, on a thread of its own.

use std::mem;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lyncast_core::rank::Timestamp;

use super::clips::Civil;
use crate::warning::warn;

const WAIT: Duration = Duration::from_secs(1);

/// The zone of the wall clock that the clipboard screen reads its days on.
#[derive(Clone, Copy)]
pub enum Zone {
    /// The system's zone, which has loaded, so `localtime_r` reads no file.
    Local,
    /// UTC, because the system's zone did not load.
    Utc,
    /// A fixed number of seconds east of UTC.
    #[cfg(test)]
    East(i64),
}

impl Zone {
    /// The system's zone, once a thread has loaded it within `WAIT`, else UTC with a warning.
    /// A thread still loading it holds glibc's zone lock, so nothing calls `localtime_r` again.
    pub fn load() -> Zone {
        let (loaded, waited) = mpsc::channel();
        let started = thread::Builder::new()
            .name("zone".to_owned())
            .spawn(move || {
                local_offset(Timestamp(0));
                let _ = loaded.send(());
            });
        let why = match started {
            Ok(_) => match waited.recv_timeout(WAIT) {
                Ok(()) => return Zone::Local,
                Err(_) => format!("did not load within {} s", WAIT.as_secs()),
            },
            Err(error) => format!("did not load: {error}"),
        };
        warn(format_args!(
            "the time zone {why}, so the palette shows times in UTC"
        ));
        Zone::Utc
    }

    /// The wall time at `at`.
    pub fn date(self, at: Timestamp) -> Civil {
        Civil::at(at, self.offset(at))
    }

    fn offset(self, at: Timestamp) -> i64 {
        match self {
            Zone::Local => local_offset(at),
            Zone::Utc => 0,
            #[cfg(test)]
            Zone::East(seconds) => seconds,
        }
    }
}

fn local_offset(at: Timestamp) -> i64 {
    let seconds = at.0.div_euclid(1000) as libc::time_t;
    // SAFETY: an all-zero tm is a valid value for localtime_r to write over.
    let mut tm: libc::tm = unsafe { mem::zeroed() };
    // SAFETY: both pointers are to locals that live through the call.
    let converted = unsafe { libc::localtime_r(&seconds, &mut tm) };
    if converted.is_null() { 0 } else { tm.tm_gmtoff }
}
