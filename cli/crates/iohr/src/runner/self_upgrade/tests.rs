//! The self-upgrade's state machine on a fake release source, installer, ledger and
//! restarter, with paused time: no network, no disk but the ledger's own test, no process.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::time::Instant;

use super::*;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

const HOUR: Duration = Duration::from_secs(3600);
const MIN: Duration = Duration::from_secs(60);

fn script_dir() -> Method {
    Method::InstallScript {
        dir: PathBuf::from("/home/u/.local/bin"),
    }
}

#[derive(Clone)]
struct Fake {
    method: Method,
    /// What the lookup answers.
    latest: Arc<Mutex<Result<Version, String>>>,
    /// What the download and its checks do.
    prepare: Arc<Mutex<Result<(), Refused>>>,
    /// What the swap does.
    install: Arc<Mutex<Result<(), String>>>,
    /// What the installed file then says.
    says: Arc<Mutex<Result<Version, String>>>,
    looks: Arc<AtomicUsize>,
    /// Whether the drain was wanted at each download.
    wanted_at_prepare: Arc<Mutex<Vec<bool>>>,
    /// When each swap ran.
    installs: Arc<Mutex<Vec<Instant>>>,
    /// Whether the drain was wanted at each install.
    wanted_at_install: Arc<Mutex<Vec<bool>>>,
    drain: Arc<Drain>,
}

impl Fake {
    fn new(method: Method, latest: &str, drain: &Arc<Drain>) -> Self {
        Self {
            method,
            latest: Arc::new(Mutex::new(Ok(v(latest)))),
            prepare: Arc::new(Mutex::new(Ok(()))),
            install: Arc::new(Mutex::new(Ok(()))),
            says: Arc::new(Mutex::new(Ok(v(latest)))),
            looks: Arc::default(),
            wanted_at_prepare: Arc::default(),
            installs: Arc::default(),
            wanted_at_install: Arc::default(),
            drain: drain.clone(),
        }
    }
    fn looks(&self) -> usize {
        self.looks.load(Ordering::SeqCst)
    }
    fn installs(&self) -> usize {
        self.installs.lock().unwrap().len()
    }
    fn prepares(&self) -> usize {
        self.wanted_at_prepare.lock().unwrap().len()
    }
}

#[allow(
    unknown_lints,
    clippy::unused_async_trait_impl,
    reason = "the trait's shape; a fake has nothing to await"
)]
impl Source for Fake {
    type Staged = ();

    fn here(&self) -> Result<Here, String> {
        Ok(Here {
            exe: PathBuf::from("/home/u/.local/bin/iohr"),
            real: PathBuf::from("/home/u/.local/bin/iohr"),
            method: self.method.clone(),
        })
    }
    async fn newest(&self, _: &Method) -> Result<Version, String> {
        self.looks.fetch_add(1, Ordering::SeqCst);
        self.latest.lock().unwrap().clone()
    }
    async fn prepare(&self, _: &Here, _: &Version) -> Result<(), Refused> {
        self.wanted_at_prepare
            .lock()
            .unwrap()
            .push(self.drain.is_wanted());
        self.prepare.lock().unwrap().clone()
    }
    async fn apply(&self, _: &Here, _: &Version, (): ()) -> Result<(), String> {
        self.installs.lock().unwrap().push(Instant::now());
        self.wanted_at_install
            .lock()
            .unwrap()
            .push(self.drain.is_wanted());
        self.install.lock().unwrap().clone()
    }
    fn installed(&self, _: &Here) -> Result<Version, String> {
        self.says.lock().unwrap().clone()
    }
}

#[derive(Clone, Default)]
struct Marks {
    last: Arc<Mutex<Option<Version>>>,
    refuse: Arc<Mutex<bool>>,
}

impl Ledger for Marks {
    fn installed_recently(&self, version: &Version) -> bool {
        self.last.lock().unwrap().as_ref() == Some(version)
    }
    fn record(&self, version: &Version) -> Result<(), String> {
        if *self.refuse.lock().unwrap() {
            return Err("disk full".into());
        }
        *self.last.lock().unwrap() = Some(version.clone());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Execs(Arc<AtomicUsize>);

impl Restarter for Execs {
    fn exec(&self, _: &Here) -> String {
        self.0.fetch_add(1, Ordering::SeqCst);
        "cannot start /home/u/.local/bin/iohr: test".into()
    }
}

struct Rig {
    up: Upgrader<Fake, Marks, Execs>,
    fake: Fake,
    marks: Marks,
    execs: Execs,
    drain: Arc<Drain>,
    policy: watch::Sender<Policy>,
    stop: watch::Sender<bool>,
}

fn rig(method: Method, current: &str, latest: &str, sup: Supervisor, mode: RestartMode) -> Rig {
    let drain = Arc::new(Drain::default());
    let fake = Fake::new(method, latest, &drain);
    let (marks, execs) = (Marks::default(), Execs::default());
    let (policy, rx) = watch::channel(Policy {
        enabled: true,
        interval: HOUR,
        restart: mode,
    });
    let (stop, _) = watch::channel(false);
    Rig {
        up: Upgrader {
            source: fake.clone(),
            ledger: marks.clone(),
            restarter: execs.clone(),
            supervisor: sup,
            current: v(current),
            policy: rx,
            drain: drain.clone(),
            drain_limit: DRAIN_LIMIT,
            jitter: || 0.0,
        },
        fake,
        marks,
        execs,
        drain,
        policy,
        stop,
    }
}

/// A run loop that is idle: it answers every drain request with "nothing leased".
fn idle_loop(drain: &Arc<Drain>) -> tokio::task::JoinHandle<()> {
    let drain = drain.clone();
    tokio::spawn(async move {
        loop {
            drain.publish(true);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

fn lines() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str)) {
    let all = Arc::new(Mutex::new(Vec::new()));
    let sink = all.clone();
    (all, move |l: &str| sink.lock().unwrap().push(l.to_owned()))
}

#[tokio::test(start_paused = true)]
async fn only_a_strictly_newer_release_is_chosen() {
    let cases = [
        ("0.1.0-alpha.27", "0.1.0-alpha.28", true),
        ("0.1.0-alpha.27", "0.1.0-alpha.27", false),
        ("0.1.0-alpha.27", "0.1.0-alpha.26", false),
        ("0.1.0-alpha.9", "0.1.0-alpha.10", true),
        ("0.1.0-alpha.99", "0.1.0", true),
        ("0.1.0", "0.1.0-rc.1", false),
        ("0.1.0", "0.1.1", true),
    ];
    for (current, latest, newer) in cases {
        let r = rig(
            script_dir(),
            current,
            latest,
            Supervisor::None,
            RestartMode::Exec,
        );
        let got = r.up.check().await;
        assert_eq!(
            matches!(got, Check::Newer(..)),
            newer,
            "{current} -> {latest}: {got:?}"
        );
        if !newer {
            assert!(matches!(got, Check::Current(_)), "{got:?}");
        }
    }
    let r = rig(
        script_dir(),
        "0.1.0",
        "0.1.1",
        Supervisor::None,
        RestartMode::Exec,
    );
    *r.fake.latest.lock().unwrap() = Err("GitHub says 503".into());
    assert_eq!(r.up.check().await, Check::Failed("GitHub says 503".into()));
}

#[tokio::test(start_paused = true)]
async fn apt_and_foreign_copies_report_and_never_install() {
    for method in [Method::Apt, Method::Other] {
        let r = rig(
            method,
            "0.1.0-alpha.27",
            "0.1.0-alpha.28",
            Supervisor::None,
            RestartMode::Exec,
        );
        assert!(matches!(r.up.check().await, Check::Skipped(..)));
        let (all, log) = lines();
        let _idle = idle_loop(&r.drain);
        let run = r.up.run(r.stop.subscribe(), &log);
        let _ = tokio::time::timeout(5 * HOUR, run).await;
        assert!(r.fake.looks() >= 4, "it kept looking: {}", r.fake.looks());
        assert_eq!(r.fake.installs(), 0);
        assert!(
            !r.drain.is_wanted(),
            "no drain for a copy that cannot upgrade"
        );
        assert_eq!(
            all.lock().unwrap().len(),
            1,
            "said once per version: {:?}",
            all.lock().unwrap()
        );
        assert!(all.lock().unwrap()[0].contains("0.1.0-alpha.28"));
    }
}

#[tokio::test(start_paused = true)]
async fn nothing_is_installed_until_the_loop_says_nothing_is_leased() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let started = Instant::now();
    let (_, log) = lines();
    let drain = r.drain.clone();
    let driver = async {
        tokio::time::sleep(10 * MIN).await;
        // Busy: a turn is running, so a drain is wanted but not complete.
        drain.publish(false);
        assert_eq!(r.fake.installs(), 0, "installed while a lease was held");
        assert!(drain.is_wanted(), "the loop is asked to take nothing");
        tokio::time::sleep(10 * MIN).await;
        assert_eq!(r.fake.installs(), 0);
        drain.publish(true);
    };
    let (got, ()) = tokio::join!(r.up.upgrade(&ver, &here, r.stop.subscribe(), &log), driver);
    let _ = got;
    let at = r.fake.installs.lock().unwrap()[0];
    assert!(
        at - started >= 20 * MIN,
        "installed after {:?}",
        at - started
    );
    assert_eq!(*r.fake.wanted_at_install.lock().unwrap(), [true]);
}

#[tokio::test(start_paused = true)]
async fn a_run_in_progress_holds_the_drain_open_and_no_run_starts_while_wanted() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let hold = r
        .drain
        .hold()
        .expect("a run may start before a drain is wanted");
    let _idle = idle_loop(&r.drain);
    let (_, log) = lines();
    let started = Instant::now();
    let drain = r.drain.clone();
    let driver = async {
        tokio::time::sleep(30 * MIN).await;
        assert!(drain.hold().is_none(), "no new run once a drain is wanted");
        assert_eq!(r.fake.installs(), 0, "installed under a run in progress");
        drop(hold);
    };
    let (_, ()) = tokio::join!(r.up.upgrade(&ver, &here, r.stop.subscribe(), &log), driver);
    assert!(r.fake.installs.lock().unwrap()[0] - started >= 30 * MIN);
    assert!(r.drain.hold().is_some(), "runs may start again afterwards");
}

#[tokio::test(start_paused = true)]
async fn work_that_never_ends_gives_up_the_round_and_takes_work_again() {
    let mut r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    r.up.drain_limit = HOUR;
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let (_, log) = lines();
    let got = r.up.upgrade(&ver, &here, r.stop.subscribe(), &log).await;
    assert!(got.unwrap_err().contains("still running after 60 minutes"));
    assert!(!r.drain.is_wanted(), "leases are taken again");
    assert_eq!(r.fake.installs(), 0);
}

#[tokio::test(start_paused = true)]
async fn shutdown_ends_the_wait_and_the_drain() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    let (_, log) = lines();
    let stop = r.stop.subscribe();
    let ended = async {
        tokio::time::sleep(5 * MIN).await;
        r.stop.send_replace(true);
    };
    // During the first wait.
    let (out, ()) = tokio::join!(r.up.run(stop, &log), ended);
    assert_eq!(out, Outcome::Stopped);
    // During the drain.
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let stop = r.stop.subscribe();
    let ended = async {
        tokio::time::sleep(5 * MIN).await;
        assert!(r.drain.is_wanted());
        r.stop.send_replace(true);
    };
    let (got, ()) = tokio::join!(r.up.upgrade(&ver, &here, stop, &log), ended);
    assert_eq!(got, Ok(None));
    assert!(!r.drain.is_wanted());
    assert_eq!(r.fake.installs(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_failed_install_keeps_the_old_version_and_backs_off() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    *r.fake.install.lock().unwrap() = Err("attestation does not verify".into());
    let _idle = idle_loop(&r.drain);
    let (all, log) = lines();
    let t0 = Instant::now();
    let run = r.up.run(r.stop.subscribe(), &log);
    let _ = tokio::time::timeout(3 * HOUR, run).await;
    // First look at 60 s, then waits of 5, 10, 20, 40, 60 minutes (never past the interval).
    let at: Vec<u64> = r
        .fake
        .installs
        .lock()
        .unwrap()
        .iter()
        .map(|t| (*t - t0).as_secs() / 60)
        .collect();
    assert_eq!(at, [1, 6, 16, 36, 76, 136], "{at:?}");
    assert_eq!(r.execs.0.load(Ordering::SeqCst), 0, "never restarted");
    assert_eq!(r.marks.last.lock().unwrap().clone(), None);
    assert!(!r.drain.is_wanted(), "leases are taken again between tries");
    let all = all.lock().unwrap();
    let failures: Vec<&String> = all
        .iter()
        .filter(|l| l.contains("upgrade failed"))
        .collect();
    assert_eq!(failures.len(), 6, "one line per failure: {all:?}");
    assert!(failures[0].contains("(1 in a row)") && failures[5].contains("(6 in a row)"));
    assert!(all.iter().all(|l| !l.to_lowercase().contains("token")));
}

#[tokio::test(start_paused = true)]
async fn the_delay_is_the_interval_then_a_back_off_that_never_passes_it() {
    assert_eq!(delay(HOUR, 0, 0.0), HOUR);
    assert_eq!(
        delay(HOUR, 0, 1.0),
        HOUR + 6 * MIN,
        "up to 10 percent of jitter"
    );
    assert_eq!(delay(HOUR, 0, 7.0), HOUR + 6 * MIN, "jitter is clamped");
    let waits: Vec<u64> = (1..=6)
        .map(|f| delay(HOUR, f, 0.0).as_secs() / 60)
        .collect();
    assert_eq!(waits, [5, 10, 20, 40, 60, 60]);
    assert_eq!(
        delay(5 * MIN, 9, 0.0),
        5 * MIN,
        "the shortest interval is the cap"
    );
    let week = Duration::from_hours(168);
    assert_eq!(
        delay(week, 40, 0.0),
        FAIL_MAX,
        "the back-off tops out at 6 hours"
    );
    assert_eq!(delay(week, u32::MAX, 0.0), FAIL_MAX, "no overflow");
}

#[tokio::test(start_paused = true)]
async fn an_install_that_says_another_version_is_not_restarted() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::Systemd,
        RestartMode::Exit,
    );
    *r.fake.says.lock().unwrap() = Ok(v("0.1.0-alpha.27"));
    let _idle = idle_loop(&r.drain);
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let (_, log) = lines();
    let got = r.up.upgrade(&ver, &here, r.stop.subscribe(), &log).await;
    assert!(got.unwrap_err().contains("the file says 0.1.0-alpha.27"));
    assert_eq!(
        *r.marks.last.lock().unwrap(),
        Some(v("0.1.0-alpha.28")),
        "another version came out: noted, so the guard holds the next rounds"
    );
    *r.marks.last.lock().unwrap() = None;
    *r.fake.says.lock().unwrap() = Err("does not run".into());
    let got = r.up.upgrade(&ver, &here, r.stop.subscribe(), &log).await;
    assert!(got.unwrap_err().contains("does not run"));
    assert_eq!(r.execs.0.load(Ordering::SeqCst), 0);
    assert_eq!(*r.marks.last.lock().unwrap(), None, "nothing was noted");
}

#[tokio::test(start_paused = true)]
async fn a_failed_download_never_stops_work_nor_waits_for_it() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    *r.fake.prepare.lock().unwrap() = Err(Refused::failed("checksum mismatch"));
    // A run is in progress and never ends: a drain before the download would idle the
    // runner for the whole DRAIN_LIMIT and only then find the failure.
    let _hold = r.drain.hold().unwrap();
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let (_, log) = lines();
    let t0 = Instant::now();
    let got = r.up.upgrade(&ver, &here, r.stop.subscribe(), &log).await;
    assert!(got.unwrap_err().contains("checksum mismatch"));
    assert!(
        Instant::now() - t0 < MIN,
        "waited {:?} for a drain before the download",
        Instant::now() - t0
    );
    assert_eq!(*r.fake.wanted_at_prepare.lock().unwrap(), [false]);
    assert!(!r.drain.is_wanted(), "work was never held back");
    assert!(r.drain.hold().is_some(), "runs may start");
    assert_eq!(r.fake.installs(), 0);
    assert_eq!(
        *r.marks.last.lock().unwrap(),
        None,
        "a plain failure is not noted"
    );
}

#[tokio::test(start_paused = true)]
async fn the_download_comes_before_the_drain_and_the_swap_after_it() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::Systemd,
        RestartMode::Exit,
    );
    let _idle = idle_loop(&r.drain);
    let (_, log) = lines();
    let out = r.up.run(r.stop.subscribe(), &log).await;
    assert_eq!(out, Outcome::Exit);
    assert_eq!(*r.fake.wanted_at_prepare.lock().unwrap(), [false]);
    assert_eq!(*r.fake.wanted_at_install.lock().unwrap(), [true]);
}

#[tokio::test(start_paused = true)]
async fn a_tap_that_lags_the_release_is_noted_once_and_not_tried_every_round() {
    // Before the install: the formula is older than the release.
    let r = rig(
        Method::Homebrew,
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exec,
    );
    *r.fake.prepare.lock().unwrap() = Err(Refused {
        why: "the tap still lists 0.1.0-alpha.27".into(),
        hold: true,
    });
    let _idle = idle_loop(&r.drain);
    let (all, log) = lines();
    let run = r.up.run(r.stop.subscribe(), &log);
    let _ = tokio::time::timeout(6 * HOUR, run).await;
    assert_eq!(
        r.fake.prepares(),
        1,
        "the guard holds after the first round"
    );
    assert_eq!(*r.marks.last.lock().unwrap(), Some(v("0.1.0-alpha.28")));
    assert_eq!(r.fake.installs(), 0);
    assert!(
        all.lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("tried in the last 24 hours")),
        "{:?}",
        all.lock().unwrap()
    );

    // After the install: brew gave the formula's version, which is not the release.
    let r = rig(
        Method::Homebrew,
        "0.1.0-alpha.27",
        "0.1.0-alpha.29",
        Supervisor::None,
        RestartMode::Exec,
    );
    *r.fake.says.lock().unwrap() = Ok(v("0.1.0-alpha.28"));
    let _idle = idle_loop(&r.drain);
    let (_, log) = lines();
    let run = r.up.run(r.stop.subscribe(), &log);
    let _ = tokio::time::timeout(6 * HOUR, run).await;
    assert_eq!(r.fake.installs(), 1, "one swap, then the guard holds");
    assert_eq!(*r.marks.last.lock().unwrap(), Some(v("0.1.0-alpha.29")));
    assert_eq!(r.execs.0.load(Ordering::SeqCst), 0, "never restarted");
    assert!(!r.drain.is_wanted());
}

#[tokio::test(start_paused = true)]
async fn on_exit_the_drain_stays_on_so_no_lease_is_taken_before_the_loop_ends() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::Systemd,
        RestartMode::Exit,
    );
    let _idle = idle_loop(&r.drain);
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let (_, log) = lines();
    let got = r.up.upgrade(&ver, &here, r.stop.subscribe(), &log).await;
    assert_eq!(got, Ok(Some(Outcome::Exit)));
    assert!(r.drain.is_wanted(), "want(false) would let a lease in");
    assert!(r.drain.hold().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_ledger_that_cannot_be_written_stops_the_restart() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::Systemd,
        RestartMode::Exit,
    );
    *r.marks.refuse.lock().unwrap() = true;
    let _idle = idle_loop(&r.drain);
    let Check::Newer(ver, here) = r.up.check().await else {
        panic!("newer expected")
    };
    let (_, log) = lines();
    let got = r.up.upgrade(&ver, &here, r.stop.subscribe(), &log).await;
    assert!(got.unwrap_err().contains("not restarting: disk full"));
}

#[tokio::test(start_paused = true)]
async fn a_restart_that_fails_is_tried_once_and_the_version_is_not_installed_again() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::Launchd {
            label: "hr.inorbit.runner".into(),
        },
        RestartMode::Exec,
    );
    let _idle = idle_loop(&r.drain);
    let (all, log) = lines();
    let run = r.up.run(r.stop.subscribe(), &log);
    let _ = tokio::time::timeout(6 * HOUR, run).await;
    assert_eq!(
        r.execs.0.load(Ordering::SeqCst),
        1,
        "exec once, then the guard holds"
    );
    assert_eq!(
        r.fake.installs(),
        1,
        "the same version is not installed again"
    );
    assert_eq!(*r.marks.last.lock().unwrap(), Some(v("0.1.0-alpha.28")));
    assert!(!r.drain.is_wanted());
    let all = all.lock().unwrap();
    assert!(
        all.iter()
            .any(|l| l.contains("restarting in place (launchd (hr.inorbit.runner))")),
        "{all:?}"
    );
    assert!(
        all.iter().any(|l| l.contains("tried in the last 24 hours")),
        "{all:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn exit_mode_exits_under_a_supervisor_and_execs_without_one() {
    for sup in [
        Supervisor::Systemd,
        Supervisor::Launchd { label: "l".into() },
    ] {
        let r = rig(
            script_dir(),
            "0.1.0-alpha.27",
            "0.1.0-alpha.28",
            sup,
            RestartMode::Exit,
        );
        let _idle = idle_loop(&r.drain);
        let (all, log) = lines();
        let out = r.up.run(r.stop.subscribe(), &log).await;
        assert_eq!(out, Outcome::Exit);
        assert_eq!(r.execs.0.load(Ordering::SeqCst), 0, "no exec in exit mode");
        assert!(all.lock().unwrap().iter().any(|l| l.contains("exiting 75")));
        assert_eq!(r.fake.installs(), 1);
    }
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::None,
        RestartMode::Exit,
    );
    let _idle = idle_loop(&r.drain);
    let (_, log) = lines();
    let run = r.up.run(r.stop.subscribe(), &log);
    let _ = tokio::time::timeout(10 * MIN, run).await;
    assert_eq!(
        r.execs.0.load(Ordering::SeqCst),
        1,
        "nobody would start it: exec"
    );
}

#[tokio::test(start_paused = true)]
async fn the_switch_turns_the_upgrade_off_and_back_on_without_a_restart() {
    let r = rig(
        script_dir(),
        "0.1.0-alpha.27",
        "0.1.0-alpha.28",
        Supervisor::Systemd,
        RestartMode::Exit,
    );
    r.policy.send_modify(|p| p.enabled = false);
    assert_eq!(r.up.check().await, Check::Disabled);
    let _idle = idle_loop(&r.drain);
    let (_, log) = lines();
    let policy = r.policy.clone();
    let flip = async {
        tokio::time::sleep(5 * HOUR).await;
        assert_eq!(r.fake.looks(), 0, "off: no release lookup at all");
        assert_eq!(r.fake.installs(), 0);
        assert!(!r.drain.is_wanted());
        policy.send_modify(|p| p.enabled = true);
    };
    let (out, ()) = tokio::join!(r.up.run(r.stop.subscribe(), &log), flip);
    assert_eq!(out, Outcome::Exit, "on again after a SIGHUP: it upgrades");
    assert_eq!(r.fake.installs(), 1);
}

#[test]
fn the_ledger_file_remembers_a_version_for_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = FileLedger {
        path: dir.path().join("runner").join("last-upgrade"),
    };
    assert!(
        !ledger.installed_recently(&v("0.1.0-alpha.28")),
        "no file yet"
    );
    ledger.record(&v("0.1.0-alpha.28")).unwrap();
    assert!(ledger.installed_recently(&v("0.1.0-alpha.28")));
    assert!(
        !ledger.installed_recently(&v("0.1.0-alpha.29")),
        "another version"
    );
    std::fs::write(&ledger.path, "0.1.0-alpha.28 1000\n").unwrap();
    assert!(!ledger.installed_recently(&v("0.1.0-alpha.28")), "long ago");
    std::fs::write(&ledger.path, "not a ledger").unwrap();
    assert!(
        !ledger.installed_recently(&v("0.1.0-alpha.28")),
        "unreadable counts as never"
    );
}
