//! The coordinator's runner (RFC 0120, "The runner protocol"): this machine's half of a
//! coordinator turn. It never runs a model itself and never holds a provider credential:
//! it starts the person's own, unmodified `claude -p` and reports what it printed.

pub(crate) mod claude;
pub(crate) mod env;
pub(crate) mod exec;
pub(crate) mod gate;
pub(crate) mod provider;
pub(crate) mod run_loop;
pub(crate) mod service;
pub(crate) mod signing;
