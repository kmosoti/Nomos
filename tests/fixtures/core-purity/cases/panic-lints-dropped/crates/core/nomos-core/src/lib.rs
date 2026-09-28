//! Negative control: the panic lints are no longer denied at the crate root.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used)]
extern crate alloc;
