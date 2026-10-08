mod assemble;
mod controls;
mod error;
mod guard;
mod hub;
mod placeholder;
mod places;
mod ports;
mod pulse;
mod routes;
mod signals;
mod slots;
mod source;
mod stage;
mod types;

pub use assemble::{assemble, ledger_for, pool_config, Assembly, Parts, PoolSettings};
pub use controls::{ControlPorts, Controls, LIMITS_KEY};
pub use error::MediaError;
pub use guard::AbortOnDrop;
pub use hub::{Deliver, Hub, Subscription};
pub use placeholder::{placeholder_for, CacheState, Placeholder};
pub use places::Places;
pub use ports::{CacheControl, PoolControl, PrefetchControl, SettingsStore, SlotDirectives};
pub use pulse::spawn_pulse;
pub use routes::Routes;
pub use signals::Signals;
pub use slots::{PlayerSlot, PlayerSlotFactory, RetainLevel, SlotCall, SlotRunner, SlotSample};
pub use source::{MediaItem, PlaybackRoute};
pub use stage::{Stage, StageParts};
pub use types::{Kind, KindUsage, MediaState, MediaStats, Pressure, ReleaseCounts, SlotCounts};

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod assemble_tests;
#[cfg(test)]
mod controls_tests;
#[cfg(test)]
mod hub_tests;
#[cfg(test)]
mod placeholder_tests;
#[cfg(test)]
mod places_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod signals_tests;
#[cfg(test)]
mod slots_tests;
