//! Whether the machine looks like it can spare the work.
//!
//! None of this observes the user. There is no accessibility permission, no
//! input monitoring, no screen recording, no window titles and no process
//! inspection here — only what the operating system already publishes about
//! its own power and load, through documented interfaces that require no
//! permission and no elevation.
//!
//! **This is not idle detection, and nothing in Scuttle calls it that.** A
//! quiet machine on mains power is not a user who has stepped away; it is a
//! reasonable moment to try, and nothing more. Every signal here is a reason
//! to *defer*, never evidence that anyone is absent.
//!
//! Aggregate idle time is deliberately not consulted. Both platforms publish
//! it without a permission prompt — `IOHIDSystem`'s `HIDIdleTime` on macOS,
//! `GetLastInputInfo` on Windows — but no honest claim can be built on a
//! number that cannot tell a reader from an empty chair, so Scuttle does not
//! collect it.

/// What could be learned about the machine, with "could not tell" spelled out
/// rather than guessed. Every field is `Option`, and every unknown is treated
/// as a reason to be careful rather than as permission to proceed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Conditions {
    /// `Some(true)` when running from mains.
    pub on_external_power: Option<bool>,
    /// `Some(true)` when the system's battery-saving mode is on.
    pub low_power_mode: Option<bool>,
    /// `Some(true)` when the system has recorded thermal pressure.
    pub thermal_pressure: Option<bool>,
    /// One-minute load average per core, where the platform publishes one.
    pub load_per_core: Option<f64>,
}

/// Above this, the machine is busy enough that Scuttle should stay out of the
/// way. Deliberately generous: this is a courtesy, not a guarantee, and a
/// check that skips a day costs nothing.
pub const BUSY_LOAD_PER_CORE: f64 = 0.7;

impl Conditions {
    /// Why this is not a good moment, if it is not.
    ///
    /// Unknown signals do not block — a desktop with no battery would
    /// otherwise never qualify — but they are never read as encouragement
    /// either. The one signal that must be positively true is external power,
    /// and only when the machine actually has a battery to be on.
    pub fn unsuitable(&self) -> Option<&'static str> {
        if self.on_external_power == Some(false) {
            return Some("on battery");
        }
        if self.low_power_mode == Some(true) {
            return Some("low power mode");
        }
        if self.thermal_pressure == Some(true) {
            return Some("the machine is running hot");
        }
        if self.load_per_core.is_some_and(|l| l > BUSY_LOAD_PER_CORE) {
            return Some("the machine is busy");
        }
        None
    }
}

/// Ask the operating system. Costs a couple of short-lived subprocesses on
/// macOS, so callers should reach this only once the cheap gates have already
/// passed — in practice a handful of times a day, not once a tick.
pub fn read() -> Conditions {
    Conditions {
        on_external_power: platform::on_external_power(),
        low_power_mode: platform::low_power_mode(),
        thermal_pressure: platform::thermal_pressure(),
        load_per_core: load_per_core(),
    }
}

/// One-minute load average divided by the number of cores.
///
/// Unix only. Windows publishes no load average, so background checks there
/// lean on power state alone — a documented difference, not an oversight.
fn load_per_core() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut averages = [0f64; 3];
        // SAFETY: `getloadavg` writes at most `nelem` doubles into the buffer.
        let filled = unsafe { libc::getloadavg(averages.as_mut_ptr(), 3) };
        if filled < 1 {
            return None;
        }
        let cores = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(1.0);
        Some(averages[0] / cores.max(1.0))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use crate::platform::util::capture;

    /// `pmset -g ps` names the current power source. Reading it needs no
    /// permission and no elevation.
    pub fn on_external_power() -> Option<bool> {
        let out = capture("pmset", &["-g", "ps"]).to_lowercase();
        if out.is_empty() {
            return None;
        }
        if out.contains("'ac power'") || out.contains("\"ac power\"") {
            return Some(true);
        }
        if out.contains("'battery power'") || out.contains("\"battery power\"") {
            return Some(false);
        }
        // A machine with no battery at all reports nothing useful here, and a
        // desktop should not be excluded for lacking one.
        if !out.contains("internalbattery") {
            return Some(true);
        }
        None
    }

    /// `lowpowermode` in `pmset -g` is 1 when Low Power Mode is on.
    pub fn low_power_mode() -> Option<bool> {
        let out = capture("pmset", &["-g"]);
        let line = out
            .lines()
            .find(|l| l.trim_start().starts_with("lowpowermode"))?;
        let value = line.split_whitespace().nth(1)?;
        Some(value != "0")
    }

    /// `pmset -g therm` records thermal and performance warnings. The absence
    /// of a recorded warning is the normal state and reads as "no pressure".
    pub fn thermal_pressure() -> Option<bool> {
        let out = capture("pmset", &["-g", "therm"]);
        if out.trim().is_empty() {
            return None;
        }
        let pressured = out.lines().any(|line| {
            let line = line.trim();
            // "Note: No thermal warning level has been recorded" is the quiet
            // case; an actual level is reported as a non-zero number.
            (line.starts_with("CPU_Speed_Limit") || line.contains("warning level"))
                && line.split_whitespace().last().is_some_and(|last| {
                    last.parse::<i64>()
                        .map(|n| n > 0 && n < 100)
                        .unwrap_or(false)
                })
        });
        Some(pressured)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    const AC_OFFLINE: u8 = 0;
    const AC_ONLINE: u8 = 1;
    const BATTERY_FLAG_NO_BATTERY: u8 = 128;
    const SYSTEM_STATUS_FLAG_SAVER_ON: u8 = 1;

    fn status() -> Option<SYSTEM_POWER_STATUS> {
        // SAFETY: `GetSystemPowerStatus` fills the struct it is given and
        // reports success; nothing is read unless it returned non-zero.
        unsafe {
            let mut raw: SYSTEM_POWER_STATUS = std::mem::zeroed();
            if GetSystemPowerStatus(&mut raw) == 0 {
                return None;
            }
            Some(raw)
        }
    }

    pub fn on_external_power() -> Option<bool> {
        let raw = status()?;
        if raw.BatteryFlag & BATTERY_FLAG_NO_BATTERY != 0 {
            return Some(true);
        }
        match raw.ACLineStatus {
            AC_ONLINE => Some(true),
            AC_OFFLINE => Some(false),
            _ => None,
        }
    }

    /// Battery Saver, which is Windows' equivalent of Low Power Mode.
    pub fn low_power_mode() -> Option<bool> {
        let raw = status()?;
        Some(raw.SystemStatusFlag & SYSTEM_STATUS_FLAG_SAVER_ON != 0)
    }

    /// Windows publishes no thermal pressure figure without a vendor driver,
    /// so Scuttle does not pretend to one.
    pub fn thermal_pressure() -> Option<bool> {
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    pub fn on_external_power() -> Option<bool> {
        None
    }
    pub fn low_power_mode() -> Option<bool> {
        None
    }
    pub fn thermal_pressure() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_that_says_nothing_is_not_taken_as_a_yes_or_a_no() {
        // Every signal unknown: a desktop with no battery, no reported
        // thermals and no load average. Scuttle should not refuse forever,
        // and it should not invent a reason to proceed either — the other
        // gates (once a day, window hidden, nothing else running) still hold.
        assert_eq!(Conditions::default().unsuitable(), None);
    }

    #[test]
    fn running_on_battery_is_reason_enough_to_wait() {
        let conditions = Conditions {
            on_external_power: Some(false),
            ..Default::default()
        };
        assert_eq!(conditions.unsuitable(), Some("on battery"));
    }

    #[test]
    fn a_saving_or_overheating_machine_is_left_alone() {
        let saving = Conditions {
            on_external_power: Some(true),
            low_power_mode: Some(true),
            ..Default::default()
        };
        assert_eq!(saving.unsuitable(), Some("low power mode"));

        let hot = Conditions {
            on_external_power: Some(true),
            thermal_pressure: Some(true),
            ..Default::default()
        };
        assert_eq!(hot.unsuitable(), Some("the machine is running hot"));
    }

    #[test]
    fn load_is_judged_per_core_rather_than_absolutely() {
        // Eight of a machine's sixteen cores busy is not a reason to wait;
        // the same load on two cores is. The caller divides, so the threshold
        // here is per-core and the test says which side of it is which.
        let quiet = Conditions {
            on_external_power: Some(true),
            load_per_core: Some(BUSY_LOAD_PER_CORE - 0.1),
            ..Default::default()
        };
        assert_eq!(quiet.unsuitable(), None);

        let busy = Conditions {
            on_external_power: Some(true),
            load_per_core: Some(BUSY_LOAD_PER_CORE + 0.1),
            ..Default::default()
        };
        assert_eq!(busy.unsuitable(), Some("the machine is busy"));
    }

    #[test]
    fn power_is_reported_before_anything_else_is_blamed() {
        // When several things are true at once the user should hear the most
        // basic one, not whichever the code happened to check first.
        let everything = Conditions {
            on_external_power: Some(false),
            low_power_mode: Some(true),
            thermal_pressure: Some(true),
            load_per_core: Some(9.0),
        };
        assert_eq!(everything.unsuitable(), Some("on battery"));
    }
}
