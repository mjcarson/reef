//! reef's store and service: the only code that touches shoal
//!
//! The rows reef keeps in shoal are its `schema`. Built with `embedded`, shoal's engine runs in
//! this process; without it, this is shoal's client half alone and builds on stable.

pub mod schema;
