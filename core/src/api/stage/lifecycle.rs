use super::Stage;
use crate::api::ports::{PoolControl, SlotDirectives};
use crate::memory::Step;
use crate::pool::{Directive, Pressure, PoolStats};
use crate::session::Event;

impl PoolControl for Stage {
    fn stats(&self) -> PoolStats {
        self.state().ring.stats()
    }

    fn set_step(&self, step: Step) {
        let mut state = self.state();
        state.ring.set_step(step);
        state.is_restore_requested = true;
    }

    fn release_under(&self, now_ms: u64, pressure: Pressure) -> Vec<Directive> {
        self.state().ring.release_under(now_ms, pressure)
    }

    fn background(&self) -> Vec<Directive> {
        let (directives, commands) = {
            let mut state = self.state();
            let directives = state.ring.background();
            let orders = state.election.set_foreground(false);
            (directives, state.commands_for(orders))
        };
        self.dispatch(commands);
        directives
    }

    fn set_slots(&self, slots: usize) {
        let commands = self.state().ring.set_slots(slots);
        self.dispatch(commands);
    }

    fn foreground(&self) {
        let (commands, keys) = {
            let mut state = self.state();
            state.ring.foreground();
            state.is_restore_requested = true;
            let orders = state.election.set_foreground(true);
            (state.commands_for(orders), state.loaded.keys().cloned().collect::<Vec<String>>())
        };
        self.dispatch(commands);
        self.inner.prefetcher.resume();
        let stage = self.clone();
        self.inner.handle.spawn(async move {
            stage.inner.routes.repair().await;
            for key in keys {
                stage.feed(&key, Event::Foreground);
            }
        });
    }
}

impl SlotDirectives for Stage {
    fn apply(&self, directives: Vec<Directive>) {
        let commands = directives
            .into_iter()
            .filter_map(|directive| match directive {
                Directive::Slot(command) => Some(command),
                Directive::StopPrefetch | Directive::DropMemoryCache => None,
            })
            .collect();
        self.dispatch(commands);
    }
}
