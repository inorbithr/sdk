//! The handshake between the run loop, the approved-run poller and the self-upgrade
//! (`runner::self_upgrade`): the upgrade asks to drain, the others stop taking work, and
//! the upgrade goes on only when nothing is left in flight.
//!
//! While `wanted`, every heartbeat declares [`super::settings::NOTHING`] so the service
//! offers no new lease, an offered one is left to expire, and the approved-run poller
//! starts no new run. The run loop publishes `loop_drained` only with every slot empty
//! (nothing accepting, fetching, running, gating or waiting in the outbox) and with the
//! drain wanted, and a run in progress holds the drain open, so a stale idle can never
//! pass for a drain. Work in flight is never cut: it is waited for.

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct State {
    wanted: bool,
    loop_drained: bool,
    holds: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Drain {
    state: watch::Sender<State>,
}

/// One approved run in progress; the drain is not complete while it is held.
#[derive(Debug)]
pub(crate) struct Hold(Arc<Drain>);

impl Drop for Hold {
    fn drop(&mut self) {
        self.0
            .state
            .send_modify(|s| s.holds = s.holds.saturating_sub(1));
    }
}

impl Drain {
    /// Asks every worker to take no new work (`true`), or to take work again (`false`).
    pub(crate) fn want(&self, on: bool) {
        self.state.send_modify(|s| {
            s.wanted = on;
            if !on {
                s.loop_drained = false;
            }
        });
    }

    pub(crate) fn is_wanted(&self) -> bool {
        self.state.borrow().wanted
    }

    /// A hold for one approved run, or `None` while a drain is wanted: the decision and
    /// the count change together, so a run never starts after the drain was seen complete.
    pub(crate) fn hold(self: &Arc<Self>) -> Option<Hold> {
        let mut taken = false;
        self.state.send_if_modified(|s| {
            taken = !s.wanted;
            if taken {
                s.holds += 1;
            }
            taken
        });
        taken.then(|| Hold(self.clone()))
    }

    /// [`Drain::hold`] on a shared drain, as a function for `Option::map`.
    pub(crate) fn hold_of(this: &Arc<Self>) -> Option<Hold> {
        this.hold()
    }

    /// Waits until a drain is wanted, the run loop has nothing leased and no approved
    /// run is held. The caller races it against shutdown.
    pub(crate) async fn until_drained(&self) {
        let mut rx = self.state.subscribe();
        // `wait_for` looks at the current value first, so an already drained runner returns.
        let _ = rx
            .wait_for(|s| s.wanted && s.loop_drained && s.holds == 0)
            .await;
    }

    /// A watcher of the `wanted` flag alone, for the run loop's `select!`.
    pub(crate) fn watch(&self) -> Wanted {
        let rx = self.state.subscribe();
        let seen = rx.borrow().wanted;
        Wanted { rx, seen }
    }

    /// Published by the run loop at the top of each turn of it.
    pub(crate) fn publish(&self, idle: bool) {
        self.state.send_if_modified(|s| {
            let drained = idle && s.wanted;
            std::mem::replace(&mut s.loop_drained, drained) != drained
        });
    }
}

/// Wakes its owner when `wanted` flips, and on no other change of the state.
#[derive(Debug)]
pub(crate) struct Wanted {
    rx: watch::Receiver<State>,
    seen: bool,
}

impl Wanted {
    /// The new value of `wanted` once it differs from the last one given. Cancel safe.
    pub(crate) async fn flipped(&mut self) -> bool {
        loop {
            if self.rx.changed().await.is_err() {
                // The drain is gone: nothing will flip again.
                std::future::pending::<()>().await;
            }
            let now = self.rx.borrow_and_update().wanted;
            if now != self.seen {
                self.seen = now;
                return now;
            }
        }
    }
}
