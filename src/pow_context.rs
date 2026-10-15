//! Process-wide FishHash context used for share validation.
//!
//! Karlsen's KHashV2 proof of work (FishHash+) needs a dataset context. The light cache
//! (~75 MB) is enough for share validation: each dataset item is derived on demand,
//! which costs a few milliseconds per share. Large pools can opt into the full
//! dataset (~4.8 GB RAM, slow to build at startup) for faster validation.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use karlsen_consensus_core::header::Header;
use karlsen_hashes::pow_hashers::FishHashContext;
use karlsen_pow::State as PowState;

static FISH_CONTEXT: OnceLock<Arc<FishHashContext>> = OnceLock::new();

/// Build the FishHash context once. Must be called before any listener accepts shares.
pub fn init_fish_context(full_dataset: bool) -> Arc<FishHashContext> {
    FISH_CONTEXT
        .get_or_init(|| {
            let started = Instant::now();
            tracing::info!(
                "Building FishHash context ({}), this may take a while",
                if full_dataset { "full dataset, ~4.8 GB" } else { "light cache, ~75 MB" }
            );
            let ctx = Arc::new(FishHashContext::new(full_dataset, None));
            tracing::info!("FishHash context ready in {:.1}s", started.elapsed().as_secs_f64());
            ctx
        })
        .clone()
}

/// Return the shared FishHash context, building the light cache lazily if `init_fish_context`
/// was never called (e.g. in tests or diagnostics).
pub fn fish_context() -> Arc<FishHashContext> {
    FISH_CONTEXT.get().cloned().unwrap_or_else(|| init_fish_context(false))
}

/// Create a karlsen-pow `State` for the given header using the shared FishHash context.
pub fn pow_state(header: &Header) -> PowState {
    PowState::new(header, fish_context())
}
