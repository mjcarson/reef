//! Rank strings: sort keys with room between any two
//!
//! The user's queue sorts by rank, its rows' shoal sort key, so a move rewrites only the row
//! that moves. Shoal's sort keys are a single `String`, compared bytewise.
//!
//! - **Greenspan's fractional indexing**, as rocicorp's `fractional-indexing` (CC0) writes it. A
//!   rank is an integer part, then a fraction. The integer part is a head letter saying how many
//!   digits follow it (`a` to `z` for 1 to 26, `Z` down to `A` for 1 to 26 below zero), then
//!   those digits. The fraction never ends in `0`, so there is always room below a rank. Digits
//!   are base 62, `0-9A-Za-z`, in ASCII order, so ranks sort as their bytes do.
//! - **The ends stay short.** A rank before the first or after the last steps the integer part
//!   rather than splitting a gap, so the 62 one-digit integers on either side of `a0` come
//!   before a rank grows a character.
//! - **One gap grows.** Each split of the same gap adds a character about every six splits, so
//!   the store spreads a queue's ranks anew, with [`spread`], when a placement would make one
//!   longer than [`LONG`].
//!
//! Unlike rocicorp's, [`between`] refuses bounds out of order rather than swapping them: the
//! store always knows which neighbour is which, and a swap would hide a bug.

use anyhow::{Context, anyhow, ensure};

/// The base-62 digits, in ASCII order, so ranks sort bytewise
const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The rank of the first entry put in an empty queue
pub const FIRST: &str = "a0";

/// The longest rank the store writes before it spreads a queue's ranks anew
pub const LONG: usize = 32;

/// The smallest integer part, which no rank may be on its own, so there is room below every rank
const SMALLEST: &str = "A00000000000000000000000000";

/// The value of one base-62 digit, or `None` for a byte that isn't one
///
/// # Arguments
///
/// * `byte` - The byte to read
fn digit(byte: u8) -> Option<usize> {
    match byte {
        b'0'..=b'9' => Some(usize::from(byte - b'0')),
        b'A'..=b'Z' => Some(usize::from(byte - b'A') + 10),
        b'a'..=b'z' => Some(usize::from(byte - b'a') + 36),
        _ => None,
    }
}

/// How long the integer part a head letter starts is, the head included, or `None` for a byte
/// that isn't a head
///
/// # Arguments
///
/// * `head` - The rank's first byte
fn integer_len(head: u8) -> Option<usize> {
    match head {
        // `a` has one digit after it and `z` 26, for the integers from zero up
        b'a'..=b'z' => Some(usize::from(head - b'a') + 2),
        // `Z` has one digit after it and `A` 26, for the integers below zero
        b'A'..=b'Z' => Some(usize::from(b'Z' - head) + 2),
        _ => None,
    }
}

/// Says why a rank is malformed, or nothing if it is a rank
///
/// # Arguments
///
/// * `rank` - The text to check
fn check(rank: &str) -> anyhow::Result<()> {
    let bytes = rank.as_bytes();
    // a rank starts with a head letter
    let head = *bytes.first().context("a rank is not empty")?;
    let len = integer_len(head)
        .ok_or_else(|| anyhow!("rank `{rank}` starts with `{}`, not a letter", head as char))?;
    // every byte after it is a base-62 digit, so a rank is ASCII and splits anywhere
    ensure!(
        bytes[1..].iter().all(|&b| digit(b).is_some()),
        "rank `{rank}` holds a byte that isn't a base-62 digit"
    );
    // the head's integer part is all there
    ensure!(
        bytes.len() >= len,
        "rank `{rank}` is shorter than its head `{}` says",
        head as char
    );
    // the smallest integer alone would leave no room below it
    ensure!(
        rank != SMALLEST,
        "rank `{rank}` is the smallest integer, which leaves no room below it"
    );
    // and a fraction ending in `0` would leave none below it either
    ensure!(
        bytes.len() == len || bytes[bytes.len() - 1] != b'0',
        "rank `{rank}` ends its fraction in `0`"
    );
    Ok(())
}

/// Whether a text is a rank this module makes and reads
///
/// # Arguments
///
/// * `rank` - The text to check
///
/// # Examples
///
/// ```
/// use reef_service::rank;
///
/// assert!(rank::is_valid("a0V"));
/// assert!(!rank::is_valid("a00"));
/// ```
pub fn is_valid(rank: &str) -> bool {
    check(rank).is_ok()
}

/// Splits a checked rank into its integer part and its fraction
///
/// # Arguments
///
/// * `rank` - A rank already checked
fn split(rank: &str) -> (&str, &str) {
    // a checked rank's head is a letter and its integer part is all there
    let len = integer_len(rank.as_bytes()[0]).unwrap_or(rank.len());
    rank.split_at(len)
}

/// A fraction strictly between two, `hi` after `lo` or `None` for no bound above
///
/// Neither ends in `0`, and `lo` sorts before `hi`, padded with zeros.
///
/// # Arguments
///
/// * `lo` - The fraction below, which may be empty
/// * `hi` - The fraction above, or `None` for none
fn midpoint(lo: &[u8], hi: Option<&[u8]>) -> Vec<u8> {
    if let Some(hi) = hi {
        // the prefix both share, `lo` padded with zeros, is kept as it is
        let mut n = 0;
        while n < hi.len() && lo.get(n).copied().unwrap_or(b'0') == hi[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = hi[..n].to_vec();
            out.extend(midpoint(lo.get(n..).unwrap_or_default(), Some(&hi[n..])));
            return out;
        }
    }
    // the first digits differ, a missing one in `lo` read as zero and in `hi` as past the last
    let low = lo.first().and_then(|&b| digit(b)).unwrap_or(0);
    let high = hi
        .and_then(|h| h.first())
        .and_then(|&b| digit(b))
        .unwrap_or(DIGITS.len());
    if high > low + 1 {
        // a digit fits between them: the one halfway, rounded up
        return vec![DIGITS[(low + high).div_ceil(2)]];
    }
    match hi {
        // the digits are consecutive and `hi` goes on: its first digit alone sorts between
        Some(hi) if hi.len() > 1 => vec![hi[0]],
        // otherwise keep `lo`'s first digit, and split what follows it from the top
        _ => {
            let mut out = vec![DIGITS[low]];
            out.extend(midpoint(lo.get(1..).unwrap_or_default(), None));
            out
        }
    }
}

/// The integer part after one, or `None` past the largest
///
/// # Arguments
///
/// * `int` - The integer part, its head included
fn increment(int: &str) -> Option<String> {
    let bytes = int.as_bytes();
    // bump the last digit that isn't the largest, zeroing those after it
    for i in (1..bytes.len()).rev() {
        let next = digit(bytes[i])? + 1;
        if next < DIGITS.len() {
            let mut out = bytes[..i].to_vec();
            out.push(DIGITS[next]);
            out.resize(bytes.len(), b'0');
            return String::from_utf8(out).ok();
        }
    }
    // every digit carried, so the head steps up, `Z` to `a` across zero
    let head = match bytes[0] {
        b'z' => return None,
        b'Z' => b'a',
        head => head + 1,
    };
    // the digits are all zeros, as many as the new head says
    let len = integer_len(head)?;
    let mut out = vec![head];
    out.resize(len, b'0');
    String::from_utf8(out).ok()
}

/// The integer part before one, or `None` below the smallest
///
/// # Arguments
///
/// * `int` - The integer part, its head included
fn decrement(int: &str) -> Option<String> {
    let bytes = int.as_bytes();
    let last = DIGITS[DIGITS.len() - 1];
    // drop the last digit that isn't zero, maxing those after it
    for i in (1..bytes.len()).rev() {
        let value = digit(bytes[i])?;
        if value > 0 {
            let mut out = bytes[..i].to_vec();
            out.push(DIGITS[value - 1]);
            out.resize(bytes.len(), last);
            return String::from_utf8(out).ok();
        }
    }
    // every digit borrowed, so the head steps down, `a` to `Z` across zero
    let head = match bytes[0] {
        b'A' => return None,
        b'a' => b'Z',
        head => head - 1,
    };
    // the digits are all the largest, as many as the new head says
    let len = integer_len(head)?;
    let mut out = vec![head];
    out.resize(len, last);
    String::from_utf8(out).ok()
}

/// Concatenates an integer part and a fraction made between two
///
/// # Arguments
///
/// * `int` - The integer part
/// * `fraction` - The fraction's bytes, all base-62 digits
fn joined(int: &str, fraction: Vec<u8>) -> String {
    // base-62 digits are ASCII, so the fraction is text
    let mut out = int.to_string();
    out.extend(fraction.into_iter().map(char::from));
    out
}

/// A rank strictly between two, either of which may be missing for no bound
///
/// # Arguments
///
/// * `lo` - The rank before, or `None` to make one before `hi`
/// * `hi` - The rank after, or `None` to make one after `lo`
///
/// # Examples
///
/// ```
/// use reef_service::rank;
///
/// assert_eq!(rank::between(None, None).unwrap(), "a0");
/// assert_eq!(rank::between(Some("a0"), Some("a1")).unwrap(), "a0V");
/// assert_eq!(rank::between(None, Some("a0")).unwrap(), "Zz");
/// ```
pub fn between(lo: Option<&str>, hi: Option<&str>) -> anyhow::Result<String> {
    // both bounds are ranks, in order
    if let Some(lo) = lo {
        check(lo)?;
    }
    if let Some(hi) = hi {
        check(hi)?;
    }
    if let (Some(lo), Some(hi)) = (lo, hi) {
        ensure!(lo < hi, "rank `{lo}` does not sort before `{hi}`");
    }
    match (lo, hi) {
        // an empty queue starts at zero
        (None, None) => Ok(FIRST.to_string()),
        (None, Some(hi)) => {
            let (int, fraction) = split(hi);
            // below the smallest integer, only a fraction under `hi`'s fits
            if int == SMALLEST {
                return Ok(joined(int, midpoint(b"", Some(fraction.as_bytes()))));
            }
            // a rank with a fraction has its integer part alone below it
            if !fraction.is_empty() {
                return Ok(int.to_string());
            }
            // otherwise the integer before it
            decrement(int).ok_or_else(|| anyhow!("no rank sorts before `{hi}`"))
        }
        (Some(lo), None) => {
            let (int, fraction) = split(lo);
            // the integer after it, or past the largest a fraction above `lo`'s
            Ok(match increment(int) {
                Some(next) => next,
                None => joined(int, midpoint(fraction.as_bytes(), None)),
            })
        }
        (Some(lo), Some(hi)) => {
            let (low, low_fraction) = split(lo);
            let (high, high_fraction) = split(hi);
            // with one integer part, the fractions are split
            if low == high {
                let fraction = midpoint(low_fraction.as_bytes(), Some(high_fraction.as_bytes()));
                return Ok(joined(low, fraction));
            }
            // an integer between them is shortest
            let next = increment(low).ok_or_else(|| anyhow!("no rank sorts after `{lo}`"))?;
            if next.as_str() < hi {
                return Ok(next);
            }
            // otherwise a fraction above `lo`'s, under its own integer part
            Ok(joined(low, midpoint(low_fraction.as_bytes(), None)))
        }
    }
}

/// `n` ranks in order strictly between two, either of which may be missing for no bound
///
/// With one bound missing they are consecutive integers, so they stay short. With both they are
/// made by halving, so none is much longer than a single rank between them.
///
/// # Arguments
///
/// * `lo` - The rank before them all, or `None` for none
/// * `hi` - The rank after them all, or `None` for none
/// * `n` - How many to make
///
/// # Examples
///
/// ```
/// use reef_service::rank;
///
/// assert_eq!(rank::spread(Some("a5"), None, 3).unwrap(), ["a6", "a7", "a8"]);
/// ```
pub fn spread(lo: Option<&str>, hi: Option<&str>, n: usize) -> anyhow::Result<Vec<String>> {
    match (n, lo, hi) {
        // none, or one between the two
        (0, _, _) => Ok(Vec::new()),
        (1, _, _) => Ok(vec![between(lo, hi)?]),
        // with nothing above, each after the last
        (_, _, None) => {
            let mut out: Vec<String> = Vec::with_capacity(n);
            for _ in 0..n {
                let next = between(out.last().map(String::as_str).or(lo), None)?;
                out.push(next);
            }
            Ok(out)
        }
        // with nothing below, each before the last, then turned around
        (_, None, Some(_)) => {
            let mut out: Vec<String> = Vec::with_capacity(n);
            for _ in 0..n {
                let next = between(None, out.last().map(String::as_str).or(hi))?;
                out.push(next);
            }
            out.reverse();
            Ok(out)
        }
        // between two, one in the middle and each half spread on its side
        (_, Some(_), Some(_)) => {
            let half = n / 2;
            let middle = between(lo, hi)?;
            let mut out = spread(lo, Some(&middle), half)?;
            out.push(middle.clone());
            out.extend(spread(Some(&middle), hi, n - half - 1)?);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FIRST, between, is_valid, spread};

    /// A small deterministic generator, so a failing run can be replayed
    struct Lcg(u64);

    impl Lcg {
        /// The next number below `bound`
        ///
        /// # Arguments
        ///
        /// * `bound` - One past the largest number it may give
        fn below(&mut self, bound: usize) -> usize {
            // Knuth's MMIX constants, keeping the high bits, which vary the most
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % bound
        }
    }

    /// A rank made between two sorts strictly between them, at any gap of a growing list
    #[test]
    fn between_sorts_strictly_between() {
        let mut lcg = Lcg(1);
        let mut ranks: Vec<String> = Vec::new();
        for _ in 0..2000 {
            // a gap anywhere, the two ends included
            let at = lcg.below(ranks.len() + 1);
            let lo = at.checked_sub(1).map(|i| ranks[i].as_str());
            let hi = ranks.get(at).map(String::as_str);
            let rank = between(lo, hi).unwrap();
            // the new rank is a rank, and sorts strictly between its neighbours
            assert!(is_valid(&rank), "{rank}");
            assert!(lo.is_none_or(|lo| lo < rank.as_str()), "{lo:?} < {rank}");
            assert!(hi.is_none_or(|hi| rank.as_str() < hi), "{rank} < {hi:?}");
            ranks.insert(at, rank);
        }
        // so the list stays sorted, every rank once
        assert!(ranks.windows(2).all(|w| w[0] < w[1]));
    }

    /// The ranks rocicorp's own tests expect, from `src/test.js`
    #[test]
    fn matches_the_reference_keys() {
        let cases: &[(Option<&str>, Option<&str>, &str)] = &[
            (None, None, "a0"),
            (None, Some("a0"), "Zz"),
            (None, Some("Zz"), "Zy"),
            (Some("a0"), None, "a1"),
            (Some("a1"), None, "a2"),
            (Some("a0"), Some("a1"), "a0V"),
            (Some("a1"), Some("a2"), "a1V"),
            (Some("a0V"), Some("a1"), "a0l"),
            (Some("Zz"), Some("a0"), "ZzV"),
            (Some("Zz"), Some("a1"), "a0"),
            (None, Some("Y00"), "Xzzz"),
            (Some("bzz"), None, "c000"),
            (Some("a0"), Some("a0V"), "a0G"),
            (Some("a0"), Some("a0G"), "a08"),
            (Some("b125"), Some("b129"), "b127"),
            (Some("a0"), Some("a1V"), "a1"),
            (Some("Zz"), Some("a01"), "a0"),
            (None, Some("a0V"), "a0"),
            (None, Some("b999"), "b99"),
            (
                None,
                Some("A000000000000000000000000001"),
                "A000000000000000000000000000V",
            ),
            (
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzy"),
                None,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzz",
            ),
            (
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzz"),
                None,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzV",
            ),
        ];
        for &(lo, hi, want) in cases {
            assert_eq!(between(lo, hi).unwrap(), want, "between {lo:?} and {hi:?}");
        }
    }

    /// Ranks put again and again at the top or the bottom grow slowly
    #[test]
    fn ends_stay_short() {
        // 200 at the top, each before the last
        let mut top = FIRST.to_string();
        for _ in 0..200 {
            top = between(None, Some(&top)).unwrap();
        }
        assert!(top.len() <= 3, "{top}");
        // and 200 at the bottom, each after the last
        let mut bottom = FIRST.to_string();
        for _ in 0..200 {
            bottom = between(Some(&bottom), None).unwrap();
        }
        assert!(bottom.len() <= 3, "{bottom}");
    }

    /// A spread is as many ranks as asked for, in order, between its bounds
    #[test]
    fn spread_makes_n_in_order() {
        for (lo, hi) in [
            (None, None),
            (Some("a5"), None),
            (None, Some("a5")),
            (Some("a0"), Some("a1")),
            (Some("a0V"), Some("a0W")),
        ] {
            for n in [0, 1, 2, 7, 100] {
                let ranks = spread(lo, hi, n).unwrap();
                assert_eq!(ranks.len(), n);
                // each is a rank, and the whole run is in order inside its bounds
                assert!(ranks.iter().all(|r| is_valid(r)), "{ranks:?}");
                assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{ranks:?}");
                assert!(lo.is_none_or(|lo| ranks.first().is_none_or(|r| lo < r.as_str())));
                assert!(hi.is_none_or(|hi| ranks.last().is_none_or(|r| r.as_str() < hi)));
            }
        }
        // after the last, they are consecutive integers, as a rebalance writes them
        assert_eq!(spread(Some("a5"), None, 3).unwrap(), ["a6", "a7", "a8"]);
        // and with no bound they start at zero
        assert_eq!(spread(None, None, 3).unwrap(), ["a0", "a1", "a2"]);
    }

    /// Malformed ranks, and bounds out of order, are refused with why
    #[test]
    fn bad_ranks_refused() {
        // rocicorp's malformed keys
        for bad in ["a00", "0", "A00000000000000000000000000"] {
            assert!(!is_valid(bad), "{bad}");
            assert!(between(Some(bad), None).is_err(), "{bad}");
            assert!(between(None, Some(bad)).is_err(), "{bad}");
        }
        // and ones it reads leniently: empty, short of its head, or holding a byte not a digit
        for bad in ["", "b0", "a-", "a0é"] {
            assert!(!is_valid(bad), "{bad}");
        }
        // rocicorp swaps bounds out of order; reef refuses them, and equal ones
        let error = between(Some("a1"), Some("a0")).unwrap_err();
        assert!(
            error.to_string().contains("does not sort before"),
            "{error}"
        );
        assert!(between(Some("a1"), Some("a1")).is_err());
    }
}
