//! `webpulse hooked` — alias of `webpulse collect --hooked` (`HookedCommand`).

use anyhow::Result;

use super::{Ctx, Kind, RawArgs};

/// One-line description.
pub const SUMMARY: &str =
    "drain the accounts flagged by the Stats::RunUser hook (= collect --hooked)";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse hooked [--job [ID]]\n\nSame as: webpulse collect --hooked";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[("job", Kind::Optional)];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Record the run in registry.jobs (adopt job ID when given)
    #[arg(long)]
    pub job: Option<Option<String>>,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    super::collect::hooked(ctx, raw)
}
