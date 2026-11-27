use super::{LoadId, Stage};
use crate::api::guard::AbortOnDrop;
use crate::api::ports::{PoolControl, SlotDirectives};
use crate::memory::{Step, StepPlan};
use crate::pool::{Directive, Pressure, PoolStats};
use crate::session::Event;
use std::sync::{Arc, PoisonError};

impl PoolControl for Stage {
    fn stats(&self) -> PoolStats {
        self.state().ring.stats()
    }

    fn set_step(&self, step: Step) {
        self.state().ring.set_step(step);
        self.inner.routes.set_readahead_percent(StepPlan::for_step(step).readahead_percent.into());
    }

    fn release_under(&self, now_ms: u64, pressure: Pressure) -> Vec<Directive> {
        let mut state = self.state();
        let directives = state.ring.release_under(now_ms, pressure);
        let (commands, remaining) = split(directives);
        self.dispatch(state, commands);
        remaining
    }

    fn background(&self) -> Vec<Directive> {
        let (state, directives, commands) = {
            let mut state = self.state();
            state.foreground.take();
            let directives = state.ring.background();
            let orders = state.election.set_foreground(false);
            let (mut commands, directives) = split(directives);
            commands.extend(state.commands_for(orders, self.now()));
            (state, directives, commands)
        };
        self.dispatch(state, commands);
        directives
    }

    fn set_slots(&self, slots: usize) {
        let mut state = self.state();
        let commands = state.ring.set_slots(slots);
        self.dispatch(state, commands);
    }

    fn foreground(&self) {
        let (inner, routes) = (Arc::downgrade(&self.inner), self.inner.routes.clone());
        let token = self.next_token();
        #[cfg(test)]
        let finished = self.inner.repair_finished.clone();
        let mut state = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        let task = self.inner.handle.spawn(async move {
            #[cfg(test)]
            let _finished = super::RepairFinished(finished);
            routes.repair().await;
            let Some(inner) = inner.upgrade() else { return };
            let stage = Stage { inner };
            let (state, commands, ids) = {
                let mut state = stage.state();
                state.ring.foreground();
                let orders = state.election.set_foreground(true);
                let commands = state.commands_for(orders, stage.now());
                let ids = state.loaded.iter().map(|(key, loaded)| LoadId { key: key.clone(), slot: loaded.slot, generation: loaded.generation }).collect::<Vec<LoadId>>();
                (state, commands, ids)
            };
            stage.dispatch(state, commands);
            for id in ids {
                stage.feed_id(&id, Event::Foreground);
            }
            let finished = {
                let mut state = stage.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.foreground.take_if(|(current, _)| *current == token)
            };
            drop(finished);
        });
        state.foreground = Some((token, AbortOnDrop::new(task)));
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
        self.dispatch(self.state(), commands);
    }
}

fn split(directives: Vec<Directive>) -> (Vec<crate::pool::Command>, Vec<Directive>) {
    let mut commands = Vec::new();
    let mut remaining = Vec::new();
    for directive in directives {
        match directive {
            Directive::Slot(command) => commands.push(command),
            other => remaining.push(other),
        }
    }
    (commands, remaining)
}
