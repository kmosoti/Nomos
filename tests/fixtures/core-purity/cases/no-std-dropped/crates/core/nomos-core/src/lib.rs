//! Negative control: the `#![no_std]` attribute is gone. A doc comment
//! mentioning #![no_std] does not count.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented)]
