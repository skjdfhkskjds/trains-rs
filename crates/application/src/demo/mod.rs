//! Demonstration workloads exposed as application commands.

mod cooperative_yield;
mod k1;

pub(crate) use cooperative_yield::CooperativeYield;
pub(crate) use k1::K1;
