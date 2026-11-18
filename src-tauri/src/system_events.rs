//! Bounded, coalescing mailbox shared by native callbacks and the recovery
//! worker. Callbacks never run native display tools, network calls or async work.
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
#[cfg(feature = "desktop")]
pub(crate) mod native;

#[derive(Default)]
pub(crate) struct WallClock(Option<std::time::SystemTime>);
impl WallClock {
    pub fn observe(&mut self, now: std::time::SystemTime) -> bool {
        self.0.replace(now).is_some_and(|previous| {
            now.duration_since(previous)
                .is_ok_and(|gap| gap >= Duration::from_secs(5))
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    Sleep,
    Wake,
    #[cfg_attr(not(windows), allow(dead_code))]
    InteractiveWake,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ScreenWake,
    Displays,
    PollGap,
}

#[derive(Clone, Default, Debug)]
pub(crate) struct State {
    pub generation: u64,
    pub power: u64,
    pub sleeps: u64,
    pub wakes: u64,
    pub gaps: u64,
    pub displays: u64,
    pub screen_wakes: u64,
    pub sleeping: bool,
    last_wake: Option<Instant>,
}

struct Inner {
    changes: watch::Sender<State>,
    handled_power: AtomicU64,
    connection_checks: watch::Sender<u64>,
}

#[derive(Clone)]
pub(crate) struct Events(Arc<Inner>);
impl Default for Events {
    fn default() -> Self {
        Self(Arc::new(Inner {
            changes: watch::channel(State::default()).0,
            handled_power: AtomicU64::new(0),
            connection_checks: watch::channel(0).0,
        }))
    }
}
impl Events {
    pub fn emit(&self, event: Event) {
        self.emit_at(event, Instant::now());
    }
    fn emit_at(&self, event: Event, now: Instant) {
        let changed = self.0.changes.send_if_modified(|s| {
            match event {
                Event::Sleep => {
                    if s.sleeping {
                        return false;
                    }
                    s.sleeping = true;
                    s.sleeps += 1;
                    s.power += 1;
                }
                Event::Wake | Event::InteractiveWake => {
                    // Windows can send both automatic and user-interactive
                    // resume. One physical wake must not restart recovery twice.
                    if !s.sleeping
                        && s.last_wake.is_some_and(|last| {
                            matches!(event, Event::InteractiveWake)
                                || now.duration_since(last) < Duration::from_secs(2)
                        })
                    {
                        return false;
                    }
                    s.sleeping = false;
                    s.last_wake = Some(now);
                    s.wakes += 1;
                    s.power += 1;
                }
                Event::PollGap => {
                    // A received native wake owns the same wall-clock gap.
                    // If only Sleep arrived, a long stalled timer still recovers
                    // from a lost wake notification instead of waiting forever.
                    if !s.sleeping
                        && s.last_wake
                            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(5))
                    {
                        return false;
                    }
                    s.gaps += 1;
                    s.sleeping = false;
                    s.last_wake = Some(now);
                    s.power += 1;
                }
                Event::Displays => s.displays += 1,
                Event::ScreenWake => s.screen_wakes += 1,
            }
            s.generation += 1;
            true
        });
        if changed
            && matches!(
                event,
                Event::Wake | Event::InteractiveWake | Event::PollGap | Event::ScreenWake
            )
        {
            self.request_connection_check();
        }
    }
    pub fn state(&self) -> State {
        self.0.changes.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<State> {
        self.0.changes.subscribe()
    }
    pub fn connection_checks(&self) -> watch::Receiver<u64> {
        self.0.connection_checks.subscribe()
    }
    pub fn request_connection_check(&self) {
        self.0
            .connection_checks
            .send_modify(|value| *value = value.wrapping_add(1));
    }
    pub fn acknowledge(&self, power: u64) {
        self.0.handled_power.store(power, Ordering::Release);
    }
    pub fn ready(&self) -> bool {
        let s = self.0.changes.borrow();
        !s.sleeping && s.power == self.0.handled_power.load(Ordering::Acquire)
    }
}

/// A short coalescing window followed by a few link-training retries. An event
/// storm cannot keep postponing the next scan or create unbounded tasks.
#[derive(Default)]
pub(crate) struct Scans {
    pub due: Option<Instant>,
    retries: u8,
    pub cause: &'static str,
}
impl Scans {
    pub fn resume(&mut self, now: Instant) {
        self.due = Some(now);
        self.retries = 3;
        self.cause = "systemWake";
    }
    pub fn screen_wake(&mut self, now: Instant) {
        self.resume(now);
        self.cause = "screenWake";
    }
    pub fn displays(&mut self, now: Instant) {
        self.schedule(now, "displayChange");
    }
    pub fn periodic(&mut self, now: Instant) {
        self.schedule(now, "periodic");
    }
    fn schedule(&mut self, now: Instant, cause: &'static str) {
        let due = now + Duration::from_millis(150);
        if self.due.is_none_or(|pending| due < pending) {
            self.cause = cause;
        }
        self.due = Some(self.due.unwrap_or(due).min(due));
        self.retries = self.retries.max(1);
    }
    pub fn completed(&mut self, now: Instant) {
        self.cause = "retry";
        self.due = if self.retries > 0 {
            self.retries -= 1;
            Some(now + Duration::from_millis(if self.retries >= 2 { 250 } else { 1000 }))
        } else {
            None
        };
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_wake_checks_immediately_without_becoming_a_system_wake() {
        let events = Events::default();
        let mut checks = events.connection_checks();
        events.emit(Event::ScreenWake);
        let state = events.state();
        assert_eq!((state.screen_wakes, state.power, state.wakes), (1, 0, 0));
        assert!(events.ready());
        assert!(checks.has_changed().unwrap());
        checks.borrow_and_update();
        events.emit(Event::Sleep);
        events.emit(Event::ScreenWake);
        assert!(events.state().sleeping);
        assert!(
            !events.ready(),
            "Screen wake cannot override real system sleep"
        );
        let now = Instant::now();
        let mut scans = Scans::default();
        scans.periodic(now);
        scans.screen_wake(now);
        scans.displays(now);
        assert_eq!(scans.due, Some(now));
        assert_eq!(scans.cause, "screenWake");
        assert!(checks.has_changed().unwrap());
    }
    #[test]
    fn coalescing_keeps_sleep_and_wake_and_gates_before_worker_runs() {
        let events = Events::default();
        assert!(events.ready());
        events.emit(Event::Sleep);
        assert!(!events.ready());
        events.emit(Event::Wake);
        assert!(!events.ready());
        let s = events.state();
        assert_eq!((s.sleeps, s.wakes, s.sleeping), (1, 1, false));
        events.acknowledge(s.power);
        assert!(events.ready());
        events.emit(Event::Wake);
        events.emit(Event::PollGap);
        assert_eq!(events.state().power, s.power);
        events.emit_at(
            Event::InteractiveWake,
            Instant::now() + Duration::from_secs(60),
        );
        assert_eq!(
            events.state().power,
            s.power,
            "Delayed Windows interactive resume is the same wake"
        );
        events.emit(Event::Sleep);
        events.emit(Event::Wake);
        events.acknowledge(s.power); // Old worker cannot acknowledge a new wake.
        assert!(!events.ready());
    }
    #[test]
    fn display_storm_is_bounded_and_never_changes_power_or_signal_source() {
        let events = Events::default();
        for _ in 0..1000 {
            events.emit(Event::Displays);
        }
        assert_eq!(events.state().displays, 1000);
        assert_eq!(events.state().power, 0);
        assert!(events.ready());
        let now = Instant::now();
        let mut scans = Scans::default();
        for ms in (0..1000).step_by(10) {
            scans.displays(now + Duration::from_millis(ms));
        }
        assert_eq!(scans.due, Some(now + Duration::from_millis(150)));
        scans.resume(now);
        scans.displays(now + Duration::from_millis(5));
        assert_eq!(scans.due, Some(now));
        for _ in 0..4 {
            scans.completed(now);
        }
        assert!(scans.due.is_none());
    }
    #[test]
    fn missing_native_wake_still_has_a_polling_fallback() {
        let events = Events::default();
        let now = Instant::now();
        events.emit_at(Event::PollGap, now);
        assert!(!events.ready());
        assert_eq!(events.state().gaps, 1);
        events.emit_at(Event::Wake, now);
        events.emit_at(Event::PollGap, now + Duration::from_secs(1));
        assert_eq!(events.state().gaps, 1);
        events.emit_at(Event::PollGap, now + Duration::from_secs(60));
        assert_eq!(events.state().gaps, 2);
        events.emit_at(Event::Sleep, now + Duration::from_secs(61));
        events.emit_at(Event::PollGap, now + Duration::from_secs(120));
        assert!(!events.state().sleeping);
        assert_eq!(events.state().gaps, 3);
    }
    #[test]
    fn wall_clock_gap_includes_sleep_but_ignores_clock_rollback() {
        let mut clock = WallClock::default();
        let now = std::time::SystemTime::now();
        assert!(!clock.observe(now));
        assert!(!clock.observe(now + Duration::from_millis(16)));
        assert!(clock.observe(now + Duration::from_secs(60)));
        assert!(!clock.observe(now));
        assert!(clock.observe(now + Duration::from_secs(120)));
        assert!(!clock.observe(now + Duration::from_secs(121)));
    }
}
