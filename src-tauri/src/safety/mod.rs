//! Safety: the reason Scuttle is allowed near anyone's files.
//!
//! Three independent layers, each of which can refuse on its own:
//!
//! * [`paths`] — path arithmetic that cannot be fooled by `..`, case or links.
//! * [`protected`] — the table of places Scuttle never goes.
//! * [`validate`] — the gate every destructive action passes through, which
//!   re-derives all of the above against the live filesystem.

pub mod assess;
pub mod paths;
pub mod protected;
pub mod validate;

pub use protected::ProtectedPaths;
pub use validate::{
    assess_live, authorize, authorize_contents, authorize_path, observe, ActionContext,
    AuthorizedTarget, Bidding,
};
