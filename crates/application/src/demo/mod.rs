//! Demonstration workloads exposed as application commands.

mod cooperative_yield;
mod k1;
mod k2;

pub(crate) use cooperative_yield::CooperativeYield;
pub(crate) use k1::K1;
pub(crate) use k2::K2;
