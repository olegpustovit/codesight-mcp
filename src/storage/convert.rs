//! Narrowing conversions for SQLite integer columns.

use anyhow::Context;

pub(crate) fn to_i64_u64(value: u64) -> anyhow::Result<i64> {
    i64::try_from(value).context("value exceeded sqlite i64 range")
}

pub(crate) fn to_i64_usize(value: usize) -> anyhow::Result<i64> {
    i64::try_from(value).context("value exceeded sqlite i64 range")
}
