//! The CPU a release is built for, from the features the target's own CPU says it has
//!
//! Nothing here names a vendor or a part. The target runs reefadm's copy, built for plain x86-64
//! so it runs on any of them, and asks its CPU through std's run-time detection, which reads
//! CPUID and checks that the OS saves the wider registers. The builder asks its own CPU the same,
//! since the flags reach the build scripts and proc macros it runs, and the built reef-serve runs
//! there to say its schema. A build's class is what both have and its nightly knows: the highest
//! x86-64 psABI level they reach, and every other feature as `+name`.

use crate::exec::{Cmd, Runner};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The features shoal's hashing (gxhash) is built with, without which reef can't run
pub const REQUIRED: [&str; 2] = ["aes", "sse2"];

/// What every x86-64 CPU has, which a build for plain x86-64 is compiled with
const BASELINE: [&str; 3] = ["fxsr", "sse", "sse2"];

/// Each x86-64 psABI level above the first, and the features it adds to the one before
const LEVELS: [(&str, &[&str]); 3] = [
    (
        "x86-64-v2",
        &[
            "cmpxchg16b",
            "lahfsahf",
            "popcnt",
            "sse3",
            "sse4.1",
            "sse4.2",
            "ssse3",
        ],
    ),
    (
        "x86-64-v3",
        &[
            "avx", "avx2", "bmi1", "bmi2", "f16c", "fma", "lzcnt", "movbe", "xsave",
        ],
    ),
    (
        "x86-64-v4",
        &["avx512bw", "avx512cd", "avx512dq", "avx512f", "avx512vl"],
    ),
];

/// Runs a macro over every x86-64 feature std detects at run time on stable, by name
macro_rules! each_feature {
    ($with:ident) => {
        $with!(
            "aes",
            "pclmulqdq",
            "rdrand",
            "rdseed",
            "sse",
            "sse2",
            "sse3",
            "ssse3",
            "sse4.1",
            "sse4.2",
            "sse4a",
            "sha",
            "sha512",
            "sm3",
            "sm4",
            "avx",
            "avx2",
            "avx512f",
            "avx512cd",
            "avx512bw",
            "avx512dq",
            "avx512vl",
            "avx512ifma",
            "avx512vbmi",
            "avx512vpopcntdq",
            "avx512vbmi2",
            "gfni",
            "vaes",
            "vpclmulqdq",
            "avx512vnni",
            "avx512bitalg",
            "avx512bf16",
            "avx512vp2intersect",
            "avx512fp16",
            "avxvnni",
            "avxifma",
            "avxneconvert",
            "avxvnniint8",
            "avxvnniint16",
            "f16c",
            "fma",
            "bmi1",
            "bmi2",
            "lzcnt",
            "tbm",
            "popcnt",
            "fxsr",
            "xsave",
            "xsaveopt",
            "xsaves",
            "xsavec",
            "cmpxchg16b",
            "kl",
            "widekl",
            "adx",
            "rtm",
            "movbe",
            "ermsb"
        )
    };
}

/// The features this machine's CPU has, as std detects them at run time
///
/// A binary built for a newer CPU than plain x86-64 says every feature it was built with is
/// there, whatever the CPU it runs on, which is why reefadm refuses to copy such a build
/// ([`compiled`]).
pub fn detected() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    #[cfg(target_arch = "x86_64")]
    {
        // each feature std can ask the CPU and the OS about
        macro_rules! detect {
            ($($name:tt),* $(,)?) => {
                $(
                    if std::arch::is_x86_feature_detected!($name) {
                        found.insert($name.to_string());
                    }
                )*
            };
        }
        each_feature!(detect);
        // and LAHF and SAHF in long mode, which x86-64-v2 needs and std doesn't detect: the low bit
        // of ECX in CPUID's leaf 0x8000_0001, where the CPU has that leaf
        let highest = std::arch::x86_64::__cpuid(0x8000_0000).eax;
        if highest >= 0x8000_0001 && std::arch::x86_64::__cpuid(0x8000_0001).ecx & 1 == 1 {
            found.insert("lahfsahf".to_string());
        }
    }
    found
}

/// The features this binary was compiled with beyond plain x86-64's
///
/// A reefadm built with `-C target-cpu=native`, as an exported `RUSTFLAGS` gives it, may die of
/// an illegal instruction on an older CPU, and would report its own features as the target's.
pub fn compiled() -> Vec<&'static str> {
    // each feature, as the compiler was told
    macro_rules! compiled {
        ($($name:tt),* $(,)?) => {
            [$(($name, cfg!(target_feature = $name))),*]
        };
    }
    each_feature!(compiled)
        .into_iter()
        .filter(|(name, on)| *on && !BASELINE.contains(name))
        .map(|(name, _)| name)
        .collect()
}

/// What a release's central builds are compiled for
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Class {
    /// What rustc builds with, such as `-C target-cpu=x86-64-v3 -C target-feature=+aes,+sha`
    pub flags: String,
    /// The psABI level, or the CPU an admin named, which the class's directory is called by
    pub level: String,
    /// The target's features the builder hasn't, which the build leaves out
    pub dropped: Vec<String>,
}

impl Class {
    /// The class a target is built for: what it and the builder have, and the builder's nightly
    /// knows
    ///
    /// # Arguments
    ///
    /// * `target` - The target's features
    /// * `builder` - The builder's features
    /// * `known` - The features the nightly accepts, if it could say
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::cpu::Class;
    /// use std::collections::BTreeSet;
    ///
    /// let set = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<BTreeSet<_>>();
    /// let target = set(&["sse", "sse2", "fxsr", "aes", "pclmulqdq", "sha"]);
    /// let builder = set(&["sse", "sse2", "fxsr", "aes", "pclmulqdq"]);
    /// let class = Class::of(&target, &builder, Some(&set(&["aes", "pclmulqdq", "sha"]))).unwrap();
    /// assert_eq!(class.flags, "-C target-cpu=x86-64 -C target-feature=+aes,+pclmulqdq");
    /// assert_eq!(class.dropped, ["sha"]);
    /// ```
    pub fn of(
        target: &BTreeSet<String>,
        builder: &BTreeSet<String>,
        known: Option<&BTreeSet<String>>,
    ) -> Result<Class> {
        // what reef can't run without, on either
        for required in REQUIRED {
            if !target.contains(required) {
                bail!(
                    "the target's CPU has no {required}, which shoal's hashing is built with, so \
                     reef can't run there"
                );
            }
            if !builder.contains(required) {
                bail!(
                    "this machine's CPU has no {required}, which shoal's hashing is built with, \
                     so it can't build reef: build on one that has it"
                );
            }
        }
        // what both have, and what only the target has, which the build can't use
        let shared: BTreeSet<&String> = target.intersection(builder).collect();
        let dropped = target.difference(builder).cloned().collect();
        // the highest level whose every feature both have
        let mut level = "x86-64";
        let mut implied: BTreeSet<&str> = BASELINE.into_iter().collect();
        for (name, adds) in LEVELS {
            if !adds
                .iter()
                .all(|feature| shared.iter().any(|have| have == feature))
            {
                break;
            }
            level = name;
            implied.extend(adds.iter().copied());
        }
        // and every other feature both have that the nightly knows, sorted
        let plus: Vec<String> = shared
            .into_iter()
            .filter(|feature| !implied.contains(feature.as_str()))
            .filter(|feature| known.is_none_or(|known| known.contains(*feature)))
            .map(|feature| format!("+{feature}"))
            .collect();
        let flags = match plus.is_empty() {
            true => format!("-C target-cpu={level}"),
            false => format!("-C target-cpu={level} -C target-feature={}", plus.join(",")),
        };
        Ok(Class {
            flags,
            level: level.to_string(),
            dropped,
        })
    }

    /// A class for a CPU an admin named, with the features shoal's hashing needs added
    ///
    /// # Arguments
    ///
    /// * `cpu` - The CPU, as rustc names it, such as `x86-64-v3`
    pub fn named(cpu: &str) -> Class {
        Class {
            flags: format!("-C target-cpu={cpu} -C target-feature=+aes,+sse2"),
            level: cpu.to_string(),
            dropped: Vec::new(),
        }
    }

    /// The name of the class's own target directory, the same for the same flags whoever builds
    ///
    /// # Examples
    ///
    /// ```
    /// use reefadm::cpu::Class;
    ///
    /// let class = Class::named("x86-64-v3");
    /// assert_eq!(class.dir(), Class::named("x86-64-v3").dir());
    /// assert!(class.dir().starts_with("cpu-x86-64-v3-"));
    /// assert_ne!(class.dir(), Class::named("x86-64-v2").dir());
    /// ```
    pub fn dir(&self) -> String {
        format!("cpu-{}-{:016x}", self.level, fnv(&self.flags))
    }

    /// The CPU the flags build for, as `-C target-cpu` names it
    pub fn cpu(&self) -> &str {
        self.flags
            .split_whitespace()
            .find_map(|word| word.strip_prefix("target-cpu="))
            .unwrap_or("x86-64")
    }

    /// The features the flags add, beyond the CPU's own
    pub fn features(&self) -> BTreeSet<String> {
        self.flags
            .split_whitespace()
            .filter_map(|word| word.strip_prefix("target-feature="))
            .flat_map(|list| list.split(','))
            .filter_map(|feature| feature.strip_prefix('+'))
            .map(str::to_string)
            .collect()
    }
}

/// A string's 64-bit FNV-1a hash, which is the same on every machine and every run
///
/// # Arguments
///
/// * `text` - The string
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The features a nightly accepts for x86-64 Linux, from the first section of what
/// `rustc --print target-features` prints
///
/// The second section is LLVM's, which holds ABI and tuning pseudo-features a build must not set.
///
/// # Arguments
///
/// * `printed` - What it printed
///
/// # Examples
///
/// ```
/// let printed = "Features supported by rustc for this target:\n    aes   - Enable AES.\n    \
///                sha   - Enable SHA.\n\nCode-generation features supported by LLVM for this \
///                target:\n    soft-float   - Use software floating point.\n";
/// let known = reefadm::cpu::known(printed);
/// assert!(known.contains("aes") && known.contains("sha") && !known.contains("soft-float"));
/// ```
pub fn known(printed: &str) -> BTreeSet<String> {
    printed
        .lines()
        .skip_while(|line| !line.starts_with("Features supported by rustc"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The features the nightly a build uses accepts, or none when it can't say yet, as in a dry
/// run before it is installed
///
/// # Arguments
///
/// * `runner` - The runner
/// * `toolchain` - The nightly
pub fn known_by(runner: &mut dyn Runner, toolchain: &str) -> Result<Option<BTreeSet<String>>> {
    // never installing the toolchain to ask it
    let printed = runner.probe(
        &Cmd::new("rustup")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .args(["run", toolchain, "rustc", "--print", "target-features"])
            .args(["--target", "x86_64-unknown-linux-gnu"]),
    )?;
    let known = known(&printed.stdout);
    match (printed.ok() && !known.is_empty(), runner.dry()) {
        (true, _) => Ok(Some(known)),
        (false, true) => Ok(None),
        (false, false) => bail!(
            "{toolchain}'s rustc didn't list its target features: {}",
            printed.stderr.trim()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{Class, compiled, detected, known};
    use std::collections::BTreeSet;

    /// A set of feature names
    ///
    /// # Arguments
    ///
    /// * `names` - The names
    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// Everything x86-64-v4 needs, and the hashing's aes
    const V4: &[&str] = &[
        "fxsr",
        "sse",
        "sse2",
        "cmpxchg16b",
        "lahfsahf",
        "popcnt",
        "sse3",
        "sse4.1",
        "sse4.2",
        "ssse3",
        "avx",
        "avx2",
        "bmi1",
        "bmi2",
        "f16c",
        "fma",
        "lzcnt",
        "movbe",
        "xsave",
        "avx512bw",
        "avx512cd",
        "avx512dq",
        "avx512f",
        "avx512vl",
        "aes",
    ];

    /// A target's detected features become the level they reach and `+` flags for the rest its
    /// builder shares and the nightly knows, and a CPU without aes is refused
    #[test]
    fn a_targets_features_become_its_level_and_the_flags_the_builder_shares() {
        // a v4 CPU with three more features, built on a CPU with only two of them
        let mut target = set(V4);
        target.extend(set(&["gfni", "vaes", "avx512fp16"]));
        let mut builder = set(V4);
        builder.extend(set(&["gfni", "vaes", "sha"]));
        let known = set(&["aes", "gfni", "vaes", "avx512fp16", "sha", "avx2"]);
        let class = Class::of(&target, &builder, Some(&known)).unwrap();
        assert_eq!(
            class.flags,
            "-C target-cpu=x86-64-v4 -C target-feature=+aes,+gfni,+vaes"
        );
        assert_eq!(class.level, "x86-64-v4");
        assert_eq!(class.dropped, ["avx512fp16"]);
        assert_eq!(class.cpu(), "x86-64-v4");
        assert_eq!(class.features(), set(&["aes", "gfni", "vaes"]));
        // a level short one feature is the level below it, that feature a `+` of its own
        let mut v3 = set(V4);
        v3.remove("avx512vl");
        let class = Class::of(&v3, &set(V4), None).unwrap();
        assert_eq!(class.level, "x86-64-v3");
        assert!(
            class.flags.contains("+avx512bw") && !class.flags.contains("+avx2"),
            "{}",
            class.flags
        );
        // a nightly that doesn't know a feature leaves it out
        let class = Class::of(&set(V4), &set(V4), Some(&set(&[]))).unwrap();
        assert_eq!(class.flags, "-C target-cpu=x86-64-v4");
        // the same features make the same class, and its directory, whoever has them
        assert_eq!(
            Class::of(&set(V4), &set(V4), None).unwrap().dir(),
            Class::of(&set(V4), &set(V4), None).unwrap().dir()
        );
        // a CPU without aes, on either side, is refused
        let mut plain = set(V4);
        plain.remove("aes");
        let error = Class::of(&plain, &set(V4), None).unwrap_err().to_string();
        assert!(error.contains("target's CPU has no aes"), "{error}");
        let error = Class::of(&set(V4), &plain, None).unwrap_err().to_string();
        assert!(error.contains("this machine's CPU has no aes"), "{error}");
    }

    /// The nightly's own features are read from the first section alone
    #[test]
    fn only_rustcs_own_features_are_known() {
        let printed = "Features supported by rustc for this target:\n    adx   - Support ADX.\n    \
                       aes   - Enable AES.\n\nCode-generation features supported by LLVM for this \
                       target:\n    retpoline   - Retpolines.\n    prefer-256-bit   - Prefer 256.\n";
        assert_eq!(known(printed), set(&["adx", "aes"]));
        assert!(known("nothing").is_empty());
    }

    /// This machine's detection finds what every x86-64 has, and a test build may be compiled for
    /// more, as an exported RUSTFLAGS makes it
    #[test]
    fn this_machine_is_detected() {
        let found = detected();
        assert!(
            found.contains("sse2") && found.contains("fxsr"),
            "{found:?}"
        );
        // whatever it was compiled with, the CPU it runs on has
        for feature in compiled() {
            assert!(found.contains(feature), "{feature}");
        }
    }
}
