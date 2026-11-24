//! Slowing repeated failed sign-ins: by the name tried, and by the address it came from
//!
//! - **Counted twice: by name and by address.** A name's failures stop a guesser spread over
//!   many addresses, and an address's stop one host spraying many names. An address is counted
//!   by its whole IPv4 address, or by an IPv6 address's /64, which one host can fill alone.
//! - **A few free, then waits that double.** A name may fail [`NAME_FREE`] times, and an
//!   address [`ADDRESS_FREE`], since many people may share one. After that the next attempt
//!   waits a second, then each failure doubles the wait, up to [`WAIT_MAX`].
//! - **Counted as it is let through.** Each attempt is counted as a failure before its password
//!   is checked, and taken back once it signs in, so a burst of guesses sent at once is judged as
//!   if they came one after another, and none waits on another's hash.
//! - **Refused before any hashing.** An attempt that must wait is refused at once, its password
//!   unchecked even if it is right, and isn't counted, so retrying early doesn't lengthen the
//!   wait.
//! - **A success clears the name,** and a key's failures are forgotten an hour after its last.
//! - **Bounded memory.** At most [`KEYS_MAX`] names and as many addresses are kept. Lapsed ones
//!   go first, then the one whose last failure is oldest.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use reef_api::user::USER_NAME_MAX;

/// How many sign-ins may fail for one name before each next one waits
pub const NAME_FREE: u32 = 5;

/// How many sign-ins may fail from one address before each next one waits
pub const ADDRESS_FREE: u32 = 20;

/// The first wait, once the free failures are used
pub const FIRST_WAIT: Duration = Duration::from_secs(1);

/// The longest wait
pub const WAIT_MAX: Duration = Duration::from_secs(5 * 60);

/// How long a key's failures are kept after its last
pub const FORGET_AFTER: Duration = Duration::from_secs(60 * 60);

/// How many names, and how many addresses, are kept at most
pub const KEYS_MAX: usize = 4096;

/// A sign-in that must wait before it is tried
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyTries {
    /// How long it must wait
    pub wait: Duration,
}

impl TooManyTries {
    /// The wait in whole seconds, rounded up and at least one, as `Retry-After` says it
    pub fn seconds(&self) -> u64 {
        let whole = self.wait.as_secs() + u64::from(self.wait.subsec_nanos() > 0);
        whole.max(1)
    }
}

impl std::fmt::Display for TooManyTries {
    /// Says the sign-in must wait, and how long
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "too many sign-ins failed for that name or from this address: try again in {} s",
            self.seconds()
        )
    }
}

impl std::error::Error for TooManyTries {}

/// How long the next attempt waits after a key's failures
///
/// # Arguments
///
/// * `failures` - How many sign-ins have failed for the key
/// * `free` - How many may fail before any waits
fn wait_after(failures: u32, free: u32) -> Duration {
    // none while the free failures last
    let Some(over) = failures.checked_sub(free) else {
        return Duration::ZERO;
    };
    // then a second, doubled for each failure after, never past the cap
    let doubled = FIRST_WAIT.saturating_mul(1u32 << over.min(20));
    doubled.min(WAIT_MAX)
}

/// The key an address's failures are counted under: an IPv4 address whole, an IPv6 one by its
/// /64
///
/// # Arguments
///
/// * `address` - The address a sign-in came from
fn address_key(address: IpAddr) -> IpAddr {
    match address.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        // the network half alone, since one host can be handed the whole /64
        IpAddr::V6(v6) => {
            let network = u128::from(v6) & !u128::from(u64::MAX);
            IpAddr::V6(Ipv6Addr::from(network))
        }
    }
}

/// The key a name's failures are counted under, cut to a length no user's name passes
///
/// # Arguments
///
/// * `name` - The name a sign-in tried
fn name_key(name: &str) -> String {
    name.chars().take(USER_NAME_MAX + 1).collect()
}

/// One key's failures
#[derive(Debug, Clone, Copy)]
struct Strikes {
    /// How many sign-ins have failed
    failures: u32,
    /// When the last one was let through
    last: Instant,
}

/// The failures counted for one kind of key
#[derive(Debug)]
struct Counts<K> {
    /// Each key's failures
    keys: HashMap<K, Strikes>,
    /// How many may fail before any waits
    free: u32,
}

impl<K: Hash + Eq + Clone> Counts<K> {
    /// No failures yet, letting `free` fail before any waits
    ///
    /// # Arguments
    ///
    /// * `free` - How many may fail before any waits
    fn new(free: u32) -> Self {
        Counts {
            keys: HashMap::new(),
            free,
        }
    }

    /// How long an attempt for a key must still wait, if it must
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    /// * `now` - The time it is tried at
    fn wait(&self, key: &K, now: Instant) -> Option<Duration> {
        let strikes = self.keys.get(key)?;
        // failures forgotten are no wait at all
        let since = now.saturating_duration_since(strikes.last);
        if since >= FORGET_AFTER {
            return None;
        }
        let wait = wait_after(strikes.failures, self.free);
        wait.checked_sub(since).filter(|left| !left.is_zero())
    }

    /// Counts an attempt for a key as a failure, until it is taken back
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    /// * `now` - The time it is let through at
    fn strike(&mut self, key: &K, now: Instant) {
        // room for one more, lapsed keys first and then the oldest
        if !self.keys.contains_key(key) && self.keys.len() >= KEYS_MAX {
            self.make_room(now);
        }
        let strikes = self.keys.entry(key.clone()).or_insert(Strikes {
            failures: 0,
            last: now,
        });
        // a key whose failures lapsed starts again
        if now.saturating_duration_since(strikes.last) >= FORGET_AFTER {
            strikes.failures = 0;
        }
        strikes.failures = strikes.failures.saturating_add(1);
        strikes.last = now;
    }

    /// Takes one failure back from a key, as a sign-in that succeeded does
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    fn take_back(&mut self, key: &K) {
        if let Some(strikes) = self.keys.get_mut(key) {
            strikes.failures = strikes.failures.saturating_sub(1);
        }
    }

    /// Forgets a key's failures
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    fn clear(&mut self, key: &K) {
        self.keys.remove(key);
    }

    /// Makes room for a key: forgets the lapsed ones, and if none lapsed, the one whose last
    /// failure is oldest
    ///
    /// # Arguments
    ///
    /// * `now` - The time it is
    fn make_room(&mut self, now: Instant) {
        // the lapsed first
        self.keys
            .retain(|_, strikes| now.saturating_duration_since(strikes.last) < FORGET_AFTER);
        if self.keys.len() < KEYS_MAX {
            return;
        }
        // else the oldest
        let oldest = self
            .keys
            .iter()
            .min_by_key(|(_, strikes)| strikes.last)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            self.keys.remove(&oldest);
        }
    }
}

/// Both kinds of key, judged together under one lock
#[derive(Debug)]
struct Both {
    /// The names tried
    names: Counts<String>,
    /// The addresses tried from
    addresses: Counts<IpAddr>,
}

/// The failed sign-ins counted lately, by name and by address
#[derive(Debug)]
pub struct Throttle {
    /// Both kinds of key
    both: Mutex<Both>,
}

impl Default for Throttle {
    /// No failures yet
    fn default() -> Self {
        Throttle {
            both: Mutex::new(Both {
                names: Counts::new(NAME_FREE),
                addresses: Counts::new(ADDRESS_FREE),
            }),
        }
    }
}

impl Throttle {
    /// Lets a sign-in be tried, counting it as a failure until [`Throttle::succeeded`] takes it
    /// back, or refuses it if its name or its address must wait
    ///
    /// # Arguments
    ///
    /// * `name` - The name it tries
    /// * `address` - The address it came from, if known
    /// * `now` - The time it is tried at
    pub fn attempt(
        &self,
        name: &str,
        address: Option<IpAddr>,
        now: Instant,
    ) -> Result<(), TooManyTries> {
        // a lock poisoned by a panic still holds the counts, which is all this reads and writes
        let mut both = self.both.lock().unwrap_or_else(PoisonError::into_inner);
        let name = name_key(name);
        let address = address.map(address_key);
        // the longer of the two waits, if either must
        let by_name = both.names.wait(&name, now);
        let by_address = address.and_then(|address| both.addresses.wait(&address, now));
        if let Some(wait) = by_name.max(by_address) {
            return Err(TooManyTries { wait });
        }
        // let through, and counted until it signs in
        both.names.strike(&name, now);
        if let Some(address) = address {
            both.addresses.strike(&address, now);
        }
        Ok(())
    }

    /// Takes back a sign-in that succeeded: its name's failures are forgotten, and its
    /// address's count loses the attempt
    ///
    /// # Arguments
    ///
    /// * `name` - The name it signed in as
    /// * `address` - The address it came from, if known
    pub fn succeeded(&self, name: &str, address: Option<IpAddr>) {
        let mut both = self.both.lock().unwrap_or_else(PoisonError::into_inner);
        both.names.clear(&name_key(name));
        if let Some(address) = address.map(address_key) {
            both.addresses.take_back(&address);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ADDRESS_FREE, Counts, FORGET_AFTER, KEYS_MAX, NAME_FREE, Throttle, TooManyTries, WAIT_MAX,
        address_key, wait_after,
    };
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    /// Waits start after the free failures, double to a cap and lapse, and an IPv6 address is
    /// counted by its /64
    #[test]
    fn waits_double_to_a_cap_and_lapse() {
        // none while the free failures last, then a second, doubling, then the cap
        assert_eq!(wait_after(0, 5), Duration::ZERO);
        assert_eq!(wait_after(4, 5), Duration::ZERO);
        assert_eq!(wait_after(5, 5), Duration::from_secs(1));
        assert_eq!(wait_after(6, 5), Duration::from_secs(2));
        assert_eq!(wait_after(9, 5), Duration::from_secs(16));
        assert_eq!(wait_after(14, 5), WAIT_MAX);
        assert_eq!(wait_after(u32::MAX, 5), WAIT_MAX);
        // a name's free failures go through, and the next waits a second
        let throttle = Throttle::default();
        let start = Instant::now();
        for _ in 0..NAME_FREE {
            throttle.attempt("ada", None, start).unwrap();
        }
        let refused = throttle.attempt("ada", None, start).unwrap_err();
        assert_eq!(refused.wait, Duration::from_secs(1));
        assert_eq!(refused.seconds(), 1);
        // a refusal isn't counted, so the wait it reports doesn't grow
        let again = throttle.attempt("ada", None, start + Duration::from_millis(500));
        assert_eq!(again.unwrap_err().wait, Duration::from_millis(500));
        // once it passes, one more goes through, and the next waits twice as long
        let later = start + Duration::from_secs(1);
        throttle.attempt("ada", None, later).unwrap();
        assert_eq!(
            throttle.attempt("ada", None, later).unwrap_err().wait,
            Duration::from_secs(2)
        );
        // another name waits for nothing
        throttle.attempt("bob", None, later).unwrap();
        // a success forgets the name's failures
        throttle.succeeded("ada", None);
        throttle.attempt("ada", None, later).unwrap();
        // and failures lapse an hour after the last
        let quiet = Throttle::default();
        for _ in 0..NAME_FREE {
            quiet.attempt("ada", None, start).unwrap();
        }
        assert!(quiet.attempt("ada", None, start).is_err());
        assert!(quiet.attempt("ada", None, start + FORGET_AFTER).is_ok());
        // and start again from one
        for _ in 1..NAME_FREE {
            quiet.attempt("ada", None, start + FORGET_AFTER).unwrap();
        }
        assert!(quiet.attempt("ada", None, start + FORGET_AFTER).is_err());
        // an IPv6 address is counted by its /64, an IPv4 one whole, and a mapped one as IPv4
        let one: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let other: IpAddr = "2001:db8:1:2:bbbb::9".parse().unwrap();
        let elsewhere: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(address_key(one), address_key(other));
        assert_ne!(address_key(one), address_key(elsewhere));
        let v4: IpAddr = "192.0.2.7".parse().unwrap();
        let mapped: IpAddr = "::ffff:192.0.2.7".parse().unwrap();
        assert_eq!(address_key(v4), v4);
        assert_eq!(address_key(mapped), v4);
        assert_ne!(address_key("192.0.2.8".parse().unwrap()), v4);
    }

    /// An address may fail more often than a name, across names, and the longer wait wins
    #[test]
    fn an_address_is_counted_across_names() {
        let throttle = Throttle::default();
        let start = Instant::now();
        let from: IpAddr = "2001:db8::1".parse().unwrap();
        let neighbour: IpAddr = "2001:db8::2".parse().unwrap();
        // a new name each time, so no name waits, until the address's free failures are used
        for n in 0..ADDRESS_FREE {
            throttle
                .attempt(&format!("guess{n}"), Some(from), start)
                .unwrap();
        }
        let refused = throttle.attempt("fresh", Some(neighbour), start);
        assert_eq!(
            refused,
            Err(TooManyTries {
                wait: Duration::from_secs(1)
            })
        );
        // from elsewhere the same name goes through
        let far: IpAddr = "198.51.100.1".parse().unwrap();
        throttle.attempt("fresh", Some(far), start).unwrap();
        // a success from the address takes its own attempt back, but the others still count
        let busy = Throttle::default();
        for n in 0..ADDRESS_FREE {
            busy.attempt(&format!("guess{n}"), Some(far), start)
                .unwrap();
        }
        assert!(busy.attempt("ada", Some(far), start).is_err());
        busy.succeeded("guess0", Some(far));
        let next = busy.attempt("ada", Some(far), start);
        assert!(next.is_ok(), "{next:?}");
        assert!(busy.attempt("bob", Some(far), start).is_err());
    }

    /// The counts never hold more than their bound, the lapsed and then the oldest going first
    #[test]
    fn the_counts_are_bounded() {
        let mut counts: Counts<u32> = Counts::new(1);
        let start = Instant::now();
        // full of keys struck in order, the first the oldest
        for key in 0..KEYS_MAX as u32 {
            counts.strike(&key, start + Duration::from_millis(u64::from(key)));
        }
        assert_eq!(counts.keys.len(), KEYS_MAX);
        // one more forgets the oldest, and keeps the rest
        let now = start + Duration::from_secs(10);
        counts.strike(&u32::MAX, now);
        assert_eq!(counts.keys.len(), KEYS_MAX);
        assert!(!counts.keys.contains_key(&0));
        assert!(counts.keys.contains_key(&1));
        // once every key has lapsed, all of them go to make room
        counts.strike(&(u32::MAX - 1), now + FORGET_AFTER * 2);
        assert_eq!(counts.keys.len(), 1);
    }
}
