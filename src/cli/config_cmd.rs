//! `webpulse config` — stub, implemented by the owning work package.

use crate::app::App;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_app: &App, _args: &Args) -> Result<i32> {
    anyhow::bail!("'config' is not implemented yet")
}
