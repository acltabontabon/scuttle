//! Which releases a given installation is offered.
//!
//! There is no channel setting. An installation's channel follows from the
//! version it is: a build with a prerelease part (`0.1.0-alpha.3`) is on the
//! alpha channel, anything else is on stable. That keeps the first
//! implementation free of channel-switching interface, and makes it impossible
//! for a stable user to end up on prereleases by clicking the wrong thing.
//!
//! * **Stable** installations are only ever offered stable releases.
//! * **Alpha** installations are offered whichever release is newest, alpha or
//!   stable — so an alpha that has been superseded by the release it was
//!   leading up to moves on to it, instead of being stranded on a channel
//!   nothing publishes to.
//! * Nobody is ever offered something that is not newer than what they have.
//!
//! Versions are compared by semver precedence, never as strings: `alpha.10`
//! is newer than `alpha.9`, and `0.1.0` is newer than every `0.1.0-alpha.N`.
//! Build metadata (`+abc`) has no precedence and is ignored.

use std::cmp::Ordering;

use semver::Version;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Alpha,
}

impl Channel {
    /// The name that goes into the manifest's file name (`stable.json`).
    pub fn slug(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Alpha => "alpha",
        }
    }
}

/// The channel an installation of `version` belongs to.
pub fn channel_for(version: &Version) -> Channel {
    if version.pre.is_empty() {
        Channel::Stable
    } else {
        Channel::Alpha
    }
}

/// Whether `remote` should be offered to an installation running `current`.
///
/// The updater plugin has a comparison of its own, and this replaces it: this
/// is the one that says out loud what the policy is, and the one that is
/// tested. It is applied twice — inside the plugin's check, and again by the
/// controller on whatever the plugin returns — so a change to either side
/// cannot quietly widen who gets what.
pub fn is_offered(current: &Version, remote: &Version) -> bool {
    if channel_for(current) == Channel::Stable && !remote.pre.is_empty() {
        return false;
    }
    remote.cmp_precedence(current) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn the_channel_follows_the_installed_version() {
        assert_eq!(channel_for(&v("0.1.0-alpha.2")), Channel::Alpha);
        assert_eq!(channel_for(&v("0.2.0-rc.1")), Channel::Alpha);
        assert_eq!(channel_for(&v("0.1.0")), Channel::Stable);
        assert_eq!(channel_for(&v("1.4.2+build.7")), Channel::Stable);
    }

    #[test]
    fn versions_are_compared_by_precedence_not_as_text() {
        // As text, "alpha.10" sorts before "alpha.9".
        assert!(is_offered(&v("0.1.0-alpha.9"), &v("0.1.0-alpha.10")));
        assert!(!is_offered(&v("0.1.0-alpha.10"), &v("0.1.0-alpha.9")));
        // And "0.9.0" is older than "0.10.0", which text gets wrong too.
        assert!(is_offered(&v("0.9.0"), &v("0.10.0")));
    }

    #[test]
    fn an_alpha_moves_on_to_the_stable_release_it_led_up_to() {
        assert!(is_offered(&v("0.1.0-alpha.5"), &v("0.1.0")));
        assert!(is_offered(&v("0.1.0-alpha.5"), &v("0.2.0")));
    }

    #[test]
    fn an_alpha_is_offered_a_newer_alpha_of_a_later_version() {
        assert!(is_offered(&v("0.1.0-alpha.5"), &v("0.2.0-alpha.1")));
    }

    #[test]
    fn a_stable_installation_is_never_offered_a_prerelease() {
        assert!(!is_offered(&v("0.1.0"), &v("0.2.0-alpha.1")));
        assert!(!is_offered(&v("0.1.0"), &v("0.1.1-rc.1")));
        assert!(!is_offered(&v("1.0.0"), &v("2.0.0-beta.3")));
    }

    #[test]
    fn a_stable_installation_is_offered_a_newer_stable_release() {
        assert!(is_offered(&v("0.1.0"), &v("0.1.1")));
        assert!(is_offered(&v("0.1.0"), &v("1.0.0")));
    }

    #[test]
    fn nobody_is_offered_the_same_version_or_an_older_one() {
        assert!(!is_offered(&v("0.1.0"), &v("0.1.0")));
        assert!(!is_offered(&v("0.2.0"), &v("0.1.0")));
        assert!(!is_offered(&v("0.1.0-alpha.3"), &v("0.1.0-alpha.3")));
        assert!(!is_offered(&v("0.1.0"), &v("0.1.0-alpha.9")));
        assert!(!is_offered(&v("0.2.0-alpha.1"), &v("0.1.0")));
    }

    #[test]
    fn build_metadata_is_not_a_reason_to_update() {
        assert!(!is_offered(&v("0.1.0"), &v("0.1.0+other")));
        assert!(!is_offered(&v("0.1.0+a"), &v("0.1.0+b")));
    }
}
