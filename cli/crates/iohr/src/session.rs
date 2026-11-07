//! The session host (RFC 0213): one process per project that hosts its Roles while a person
//! has iohr open.
//!
//! Opening iohr in a project joins the host (starting it when none runs); the host stops when
//! the last session leaves, or on `iohr stop`, or on SIGTERM. What it works from is a durable
//! queue ([`queue`]) in one `DuckDB` file that only the host opens, so nothing is lost when it
//! stops, is killed or the machine restarts: a task whose lease ran out is taken again.
//!
//! - [`lifecycle`]: where a project's host keeps its files, and the lock that says one runs.
//! - [`link`]: the unix socket the shells talk to the host over (their open connection is the
//!   session; it closing, however it closes, is the session leaving).
//! - [`host`]: the host's loop.
//! - [`pool`]: the Roles at work.
//! - [`queue`]: the durable task queue.

pub(crate) mod host;
pub(crate) mod lifecycle;
pub(crate) mod link;
pub(crate) mod pool;
pub(crate) mod queue;
