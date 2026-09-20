//! When, if ever, to look around.
//!
//! All of the judgement lives in [`decide`], which is a pure function of the
//! clock, the persisted state and what the machine reports. The thread in
//! `super` does nothing but call it and act on the answer, which is what
//! makes the policy testable against a clock that can be moved by hand.
//!
//! The bias throughout is towards doing nothing. A skipped day costs a user
//! nothing at all; a check that fires while they are working costs them the
//! thing this feature was supposed to protect.

use crate::storage::{BackgroundState, Settings};

use super::conditions::Conditions;

/// How often the scheduler wakes to ask the question. Long enough that the
/// wakeups are irrelevant, short enough that "pause" and "quit" take effect
/// promptly — and it waits on a condition variable, so both of those
/// interrupt it rather than waiting this out.
pub const TICK: i64 = 5 * 60;

/// The gap between completed checks. Twenty rather than twenty-four so that a
/// machine used on a daily rhythm does not drift a check later every day
/// until it falls outside the hours it is switched on.
pub const BETWEEN_CHECKS: i64 = 20 * 60 * 60;

/// Nothing happens for this long after launch. Starting Scuttle is not a
/// reason to scan, and a login-item start least of all.
pub const SETTLE_AFTER_LAUNCH: i64 = 10 * 60;

/// Nothing happens for this long after the machine appears to have woken.
pub const SETTLE_AFTER_WAKE: i64 = 15 * 60;

/// A gap larger than this between ticks means the scheduler was not running —
/// the machine slept, or was suspended. Twice the tick plus a minute, so that
/// ordinary scheduling jitter never reads as a sleep.
pub const SLEEP_GAP: i64 = 2 * TICK + 60;

/// The longest a single check may run before it is asked to stop. A check
/// that cannot finish inside this is one that should not have been unattended
/// in the first place; it ends partial, and a partial check says nothing.
pub const GLANCE_BUDGET_SECS: u64 = 10 * 60;

/// Notices older than this are forgotten, so something that comes back after
/// half a year is worth mentioning again.
pub const NOTICE_MEMORY: i64 = 180 * 24 * 60 * 60;

/// Whether to look around on this tick.
///
/// Sweeping the drawer is not in here: it answers a different question, is
/// decided by [`should_sweep`], and happens whether or not a check does. One
/// is about finding things; the other is about keeping a promise already made
/// about how long something stays recoverable.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Not now, and why. The reason is for the log and for the interface, and
    /// is phrased so it can be shown to a person.
    Skip(Reason),
    /// Run a background check.
    Glance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Background checks are switched off.
    NotWanted,
    /// Paused until a chosen time.
    Paused,
    /// A check has already run recently.
    Recently,
    /// Too soon after launch.
    Settling,
    /// The machine seems to have just woken.
    JustWoke,
    /// Someone is looking at the window.
    Watching,
    /// Scuttle is already doing something.
    Busy,
    /// The user has not done a rummage of their own yet.
    NeverRummaged,
    /// Nowhere to look.
    NoRoots,
    /// The machine is not in a state to spare it.
    Conditions(&'static str),
}

impl Reason {
    /// Phrasing for the interface and the tray. Plain, and never a complaint.
    pub fn say(&self) -> String {
        match self {
            Reason::NotWanted => "Background checks are off.".into(),
            Reason::Paused => "Paused.".into(),
            Reason::Recently => "Looked recently.".into(),
            Reason::Settling => "Just started.".into(),
            Reason::JustWoke => "The machine just woke.".into(),
            Reason::Watching => "You are using Scuttle.".into(),
            Reason::Busy => "Scuttle is busy.".into(),
            Reason::NeverRummaged => "Waiting for a rummage of your own first.".into(),
            Reason::NoRoots => "There is nowhere to look.".into(),
            Reason::Conditions(why) => format!("Waiting — {why}."),
        }
    }
}

/// Everything the decision needs that is not the clock or the settings.
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    /// When the process started.
    pub launched_unix: i64,
    /// Whether a window is currently on screen.
    pub window_visible: bool,
    /// Whether the operation gate is held by anything at all.
    pub busy: bool,
    /// Whether any scan root resolves to somewhere real.
    pub has_roots: bool,
}

/// The whole policy.
///
/// Ordered so that the cheap answers come first: the expensive signals —
/// which cost a subprocess on macOS — are only consulted once everything free
/// has already agreed. In practice that means power is read a handful of
/// times a day rather than twelve times an hour.
pub fn decide(
    now: i64,
    settings: &Settings,
    state: &BackgroundState,
    moment: &Moment,
    // Read lazily, because reading it is the expensive part.
    conditions: impl FnOnce() -> Conditions,
) -> Decision {
    // The drawer is swept whenever the scheduler runs at all, including when
    // checks themselves are switched off. Retention is a promise about how
    // long something stays recoverable, and it should not quietly depend on
    // how often the user happens to restart the application.
    if !settings.background_checks {
        return Decision::Skip(Reason::NotWanted);
    }
    if state.paused_until_unix > now {
        return Decision::Skip(Reason::Paused);
    }
    if now - state.last_completed_unix < BETWEEN_CHECKS {
        return Decision::Skip(Reason::Recently);
    }
    if now - moment.launched_unix < SETTLE_AFTER_LAUNCH {
        return Decision::Skip(Reason::Settling);
    }
    // A long gap between ticks means time passed without the scheduler in it:
    // the machine slept, or was suspended. The answer is to settle, not to
    // catch up — there is no queue of missed checks here, and there never will
    // be. The settling period outlasts a single tick deliberately, so that a
    // machine which has just woken is left alone for a while rather than
    // scanned five minutes later.
    if slept(now, state) || now - state.woke_unix < SETTLE_AFTER_WAKE {
        return Decision::Skip(Reason::JustWoke);
    }
    if moment.window_visible {
        return Decision::Skip(Reason::Watching);
    }
    if moment.busy {
        return Decision::Skip(Reason::Busy);
    }
    if !settings.has_rummaged_before {
        return Decision::Skip(Reason::NeverRummaged);
    }
    if !moment.has_roots {
        return Decision::Skip(Reason::NoRoots);
    }
    if let Some(why) = conditions().unsuitable() {
        return Decision::Skip(Reason::Conditions(why));
    }
    Decision::Glance
}

/// Whether time seems to have passed without the scheduler in it.
///
/// A clock that moved backwards counts too: rather than reason about which
/// direction the correction went, Scuttle waits.
pub fn slept(now: i64, state: &BackgroundState) -> bool {
    if state.last_tick_unix == 0 {
        // A brand-new process. Not a sleep — the settling period after launch
        // is what covers this case, and it has its own reason.
        return false;
    }
    // Outside the range a normally-running scheduler produces: too long a gap
    // means it was not running, and a negative one means the clock moved.
    let gap = now - state.last_tick_unix;
    !(0..=SLEEP_GAP).contains(&gap)
}

/// Whether this tick should sweep the drawer, regardless of what `decide`
/// concluded about looking around.
///
/// Separate from [`decide`] because it answers a different question and has a
/// different reason to exist: one is about finding things, the other is about
/// keeping a promise already made to the user about how long something stays
/// recoverable.
pub fn should_sweep(moment: &Moment) -> bool {
    !moment.busy
}

/// When the next completed check becomes possible, as a timestamp.
pub fn eligible_at(state: &BackgroundState) -> i64 {
    let after_gap = state.last_completed_unix + BETWEEN_CHECKS;
    after_gap.max(state.paused_until_unix)
}

/// The start of the next day, local time, for "pause until tomorrow".
///
/// Takes the offset as an argument rather than reading the clock, so the
/// behaviour at a day boundary can be tested rather than hoped for.
pub fn tomorrow_unix(now: i64, utc_offset_secs: i32) -> i64 {
    const DAY: i64 = 24 * 60 * 60;
    let local = now + utc_offset_secs as i64;
    let start_of_local_day = local.div_euclid(DAY) * DAY;
    start_of_local_day + DAY - utc_offset_secs as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600;

    fn settings() -> Settings {
        Settings {
            background_mode: true,
            background_checks: true,
            has_rummaged_before: true,
            ..Default::default()
        }
    }

    fn moment(now: i64) -> Moment {
        Moment {
            launched_unix: now - HOUR,
            window_visible: false,
            busy: false,
            has_roots: true,
        }
    }

    /// A state that has been ticking along happily and is due a check.
    fn due(now: i64) -> BackgroundState {
        BackgroundState {
            last_completed_unix: now - BETWEEN_CHECKS - 1,
            last_tick_unix: now - TICK,
            ..Default::default()
        }
    }

    fn free() -> Conditions {
        Conditions {
            on_external_power: Some(true),
            low_power_mode: Some(false),
            thermal_pressure: Some(false),
            load_per_core: Some(0.1),
        }
    }

    #[test]
    fn a_machine_that_is_free_and_overdue_gets_looked_at() {
        let now = 1_700_000_000;
        assert_eq!(
            decide(now, &settings(), &due(now), &moment(now), free),
            Decision::Glance
        );
    }

    #[test]
    fn nothing_happens_when_the_setting_is_off() {
        let now = 1_700_000_000;
        let settings = Settings {
            background_checks: false,
            ..settings()
        };
        assert_eq!(
            decide(now, &settings, &due(now), &moment(now), free),
            Decision::Skip(Reason::NotWanted)
        );
    }

    #[test]
    fn a_second_check_does_not_follow_the_first_the_same_day() {
        let now = 1_700_000_000;
        let state = BackgroundState {
            last_completed_unix: now - HOUR,
            last_tick_unix: now - TICK,
            ..Default::default()
        };
        assert_eq!(
            decide(now, &settings(), &state, &moment(now), free),
            Decision::Skip(Reason::Recently)
        );
    }

    #[test]
    fn restarting_does_not_earn_a_fresh_check() {
        // The whole point of persisting `last_completed_unix`: quitting and
        // reopening must not be a way to make Scuttle scan again, deliberately
        // or otherwise.
        let now = 1_700_000_000;
        let state = BackgroundState {
            last_completed_unix: now - HOUR,
            // A brand-new process: no tick has happened yet.
            last_tick_unix: 0,
            ..Default::default()
        };
        let fresh = Moment {
            launched_unix: now - SETTLE_AFTER_LAUNCH - 1,
            ..moment(now)
        };
        assert_eq!(
            decide(now, &settings(), &state, &fresh, free),
            Decision::Skip(Reason::Recently)
        );
    }

    #[test]
    fn launching_is_never_itself_a_reason_to_scan() {
        let now = 1_700_000_000;
        let just_started = Moment {
            launched_unix: now - 30,
            ..moment(now)
        };
        assert_eq!(
            decide(now, &settings(), &due(now), &just_started, free),
            Decision::Skip(Reason::Settling)
        );
    }

    #[test]
    fn waking_from_sleep_settles_instead_of_catching_up() {
        // Three days asleep and a check long overdue. The answer is still no:
        // there is no backlog to work through, and a machine that has just
        // woken has better things to do.
        let now = 1_700_000_000;
        let state = BackgroundState {
            last_completed_unix: now - 3 * 24 * HOUR,
            last_tick_unix: now - 3 * 24 * HOUR,
            ..Default::default()
        };
        assert_eq!(
            decide(now, &settings(), &state, &moment(now), free),
            Decision::Skip(Reason::JustWoke)
        );
    }

    #[test]
    fn a_machine_that_just_woke_is_left_alone_for_longer_than_one_tick() {
        // The sleep is noticed on the first tick after waking. The danger is
        // the *second* tick five minutes later, when the gap looks normal
        // again — without the settling window, waking a laptop would be
        // followed by a scan almost immediately.
        let now = 1_700_000_000;
        let woken = BackgroundState {
            last_completed_unix: now - 3 * 24 * HOUR,
            last_tick_unix: now - TICK,
            woke_unix: now - TICK,
            ..Default::default()
        };
        assert_eq!(
            decide(now, &settings(), &woken, &moment(now), free),
            Decision::Skip(Reason::JustWoke)
        );

        // And once the settling window has passed, it proceeds.
        let later = now + SETTLE_AFTER_WAKE;
        let settled = BackgroundState {
            last_tick_unix: later - TICK,
            ..woken
        };
        assert_eq!(
            decide(later, &settings(), &settled, &moment(now), free),
            Decision::Glance
        );
    }

    #[test]
    fn a_fresh_process_is_not_mistaken_for_a_machine_waking() {
        // `last_tick_unix` is zero on a first run. That is a launch, which has
        // its own settling period and its own reason; calling it a sleep would
        // report the wrong thing to the user.
        let now = 1_700_000_000;
        let fresh = BackgroundState {
            last_tick_unix: 0,
            ..due(now)
        };
        assert!(!slept(now, &fresh));
    }

    #[test]
    fn a_clock_that_moves_backwards_is_waited_out() {
        let now = 1_700_000_000;
        let state = BackgroundState {
            last_tick_unix: now + HOUR,
            ..due(now)
        };
        assert!(slept(now, &state));
        assert_eq!(
            decide(now, &settings(), &state, &moment(now), free),
            Decision::Skip(Reason::JustWoke)
        );
    }

    #[test]
    fn nothing_starts_underneath_someone_who_is_looking() {
        let now = 1_700_000_000;
        let watching = Moment {
            window_visible: true,
            ..moment(now)
        };
        assert_eq!(
            decide(now, &settings(), &due(now), &watching, free),
            Decision::Skip(Reason::Watching)
        );
    }

    #[test]
    fn a_busy_scuttle_is_left_alone_rather_than_queued_behind() {
        // Skipping and waiting for the next tick, not retrying in a loop.
        let now = 1_700_000_000;
        let busy = Moment {
            busy: true,
            ..moment(now)
        };
        assert_eq!(
            decide(now, &settings(), &due(now), &busy, free),
            Decision::Skip(Reason::Busy)
        );
    }

    #[test]
    fn scuttle_waits_for_a_rummage_of_the_users_own_first() {
        let now = 1_700_000_000;
        let settings = Settings {
            has_rummaged_before: false,
            ..settings()
        };
        assert_eq!(
            decide(now, &settings, &due(now), &moment(now), free),
            Decision::Skip(Reason::NeverRummaged)
        );
    }

    #[test]
    fn the_expensive_signals_are_not_read_unless_everything_free_agrees() {
        // Reading power state costs a subprocess on macOS. If this regresses,
        // it becomes twelve subprocesses an hour on a machine that was never
        // going to scan anyway.
        let now = 1_700_000_000;
        let watching = Moment {
            window_visible: true,
            ..moment(now)
        };
        let mut read = false;
        let conditions = || {
            read = true;
            free()
        };
        decide(now, &settings(), &due(now), &watching, conditions);
        assert!(!read, "conditions were read for a decision already made");
    }

    #[test]
    fn a_laptop_on_battery_is_left_to_its_battery() {
        let now = 1_700_000_000;
        let on_battery = || Conditions {
            on_external_power: Some(false),
            ..free()
        };
        assert_eq!(
            decide(now, &settings(), &due(now), &moment(now), on_battery),
            Decision::Skip(Reason::Conditions("on battery"))
        );
    }

    #[test]
    fn the_drawer_is_swept_even_when_checks_are_off() {
        // Retention is a promise about how long something stays recoverable.
        // It must not depend on whether the user also wanted ambient scanning.
        let now = 1_700_000_000;
        assert!(should_sweep(&moment(now)));
        let busy = Moment {
            busy: true,
            ..moment(now)
        };
        assert!(!should_sweep(&busy), "a sweep must not barge in");
    }

    #[test]
    fn pausing_until_tomorrow_lands_at_the_start_of_the_next_local_day() {
        // 2023-11-14T22:13:20Z. In UTC the next day starts at midnight UTC.
        let now = 1_700_000_000;
        let next = tomorrow_unix(now, 0);
        assert!(next > now);
        assert_eq!(next % (24 * HOUR), 0);
        assert!(next - now < 24 * HOUR);

        // The same instant, somewhere an hour ahead: still the next local
        // midnight, and still less than a day away.
        let shifted = tomorrow_unix(now, HOUR as i32);
        assert!(shifted > now);
        assert!(shifted - now < 24 * HOUR);
        assert_ne!(shifted, next);
    }

    #[test]
    fn a_paused_scheduler_stays_paused_until_the_time_it_was_given() {
        let now = 1_700_000_000;
        let state = BackgroundState {
            paused_until_unix: now + HOUR,
            ..due(now)
        };
        assert_eq!(
            decide(now, &settings(), &state, &moment(now), free),
            Decision::Skip(Reason::Paused)
        );
        // An hour later, with the scheduler having gone on ticking throughout:
        // the pause has expired and there is nothing else in the way.
        let later = now + HOUR + 1;
        let still_ticking = BackgroundState {
            last_tick_unix: later - TICK,
            ..state
        };
        assert_eq!(
            decide(later, &settings(), &still_ticking, &moment(now), free),
            Decision::Glance
        );
    }

    #[test]
    fn nothing_scuttle_says_about_waiting_reads_as_a_complaint() {
        // The same rule the interface copy is held to: no urgency, no
        // capitals, no manufactured problems.
        let all = [
            Reason::NotWanted,
            Reason::Paused,
            Reason::Recently,
            Reason::Settling,
            Reason::JustWoke,
            Reason::Watching,
            Reason::Busy,
            Reason::NeverRummaged,
            Reason::NoRoots,
            Reason::Conditions("on battery"),
        ];
        for reason in all {
            let said = reason.say();
            assert!(!said.contains('!'), "{said}");
            assert!(
                !said
                    .chars()
                    .collect::<Vec<_>>()
                    .windows(4)
                    .any(|w| w.iter().all(|c| c.is_ascii_uppercase())),
                "{said}"
            );
            assert!(said.ends_with('.'), "{said}");
        }
    }
}
