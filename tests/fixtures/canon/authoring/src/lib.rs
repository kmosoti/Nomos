//! Shared by the two generators: the IR of a Canon, in the profile named
//! by the first argument, to standard output.

use std::io::Write;

use nomos_canon::artifact::{Profile, encode};
use nomos_canon::model::Canon;

/// Writes `canon` in the profile named by the first argument, `cbor` or
/// `jcs`, and exits non-zero on anything else.
pub fn emit(canon: &Canon) -> std::process::ExitCode {
    let profile = match std::env::args().nth(1).as_deref() {
        Some("cbor") => Profile::Cbor,
        Some("jcs") => Profile::Jcs,
        _ => return std::process::ExitCode::from(2),
    };
    let bytes = encode(canon, profile);
    match std::io::stdout().lock().write_all(&bytes) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}
