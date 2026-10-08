use super::Stage;
use crate::api::ports::{PoolControl, SlotDirectives};
use crate::memory::Step;
use crate::pool::{Directive, Pressure, Stats};
use crate::session::Event;

impl PoolControl for Stage {
    fn stats(&self) -> Stats {
        self.state().ring.stats()
    }

    fn set_step(&self, step: Step) {
        self.state().ring.set_step(step);
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
            let orders = state.election.set_foreground(true);
            (state.commands_for(orders), state.loaded.keys().cloned().collect::<Vec<String>>())
        };
        self.dispatch(commands);
        for key in keys {
            self.feed(&key, Event::Foreground);
        }
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
