//! The Cell's composition: the adapters wired to the ports the application
//! and the Substrate read through, and the commands an operator runs
//! ([`cli`], [`render`]).

pub mod cli;
pub mod render;

use nomos_core::resource::Digest;
use nomos_store::ContentStore;
use nomos_substrate::ContentSource;

/// The Cell's content store as the Substrate's content source (ADR 0017
/// §2). A blob the store reports as corrupt is not returned: the Substrate
/// refuses an exact requirement whose content it cannot get, rather than
/// writing bytes that are not what the Canon named.
#[derive(Debug)]
pub struct StoreSource<S>(pub S);

impl<S: ContentStore> ContentSource for StoreSource<S> {
    fn bytes(&self, digest: &Digest) -> Option<Vec<u8>> {
        self.0.get(digest).ok().flatten()
    }
}
