use super::command::{Answer, Declared, Fold, Merge, OwnerKind, Query};
use super::limits::IngressLimits;
use crate::api::MediaError;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::{oneshot, watch, Notify};

struct Entry {
    command: Declared,
    bytes: u64,
    fold: Option<Fold>,
    owner: Option<u64>,
}

struct FoldSlot {
    seq: u64,
    epoch: u64,
}

struct Owner {
    kind: OwnerKind,
    is_closed: bool,
}

/// A removed command whose memory is still reserved: the executor frees it, never the caller or the lock holder.
pub(crate) struct Garbage {
    command: Declared,
    bytes: u64,
    counted: bool,
}

enum Phase {
    Open,
    Ended(MediaError),
}

struct Inner {
    seq: u64,
    epoch: u64,
    entries: BTreeMap<u64, Entry>,
    folds: HashMap<Fold, FoldSlot>,
    by_owner: HashMap<u64, Vec<u64>>,
    owners: HashMap<u64, Owner>,
    garbage: Vec<Garbage>,
    counted: usize,
    bytes: u64,
    phase: Phase,
}

impl Inner {
    /// Drops accounting. A mismatch while open is a defect: the layer ends with an error instead of hiding it.
    fn reduce(&mut self, bytes: u64, counted: usize) {
        match (self.bytes.checked_sub(bytes), self.counted.checked_sub(counted)) {
            (Some(left), Some(entries)) => {
                self.bytes = left;
                self.counted = entries;
            }
            _ => {
                if matches!(self.phase, Phase::Open) {
                    self.phase = Phase::Ended(MediaError::Unexpected { detail: "ingress accounting mismatch".to_string() });
                }
                self.bytes = self.bytes.saturating_sub(bytes);
                self.counted = self.counted.saturating_sub(counted);
            }
        }
    }

    /// Moves a queued entry to the garbage list and clears every index that pointed at it.
    fn discard(&mut self, seq: u64) {
        let Some(entry) = self.entries.remove(&seq) else { return };
        if let Some(fold) = &entry.fold {
            if self.folds.get(fold).is_some_and(|slot| slot.seq == seq) {
                self.folds.remove(fold);
            }
        }
        if let Some(owner) = entry.owner {
            if let Some(seqs) = self.by_owner.get_mut(&owner) {
                seqs.retain(|queued| *queued != seq);
                if seqs.is_empty() {
                    self.by_owner.remove(&owner);
                }
            }
        }
        self.garbage.push(Garbage { command: entry.command, bytes: entry.bytes, counted: true });
    }
}

/// An entry taken for execution. Its reservation moves to the executor state that keeps the data.
pub(crate) struct Popped {
    pub command: Declared,
    pub reserve: Reserve,
    pub owner: Option<u64>,
    pub slot: EntryPlace,
}

pub(crate) enum Upcoming {
    Release(u64, OwnerKind),
    Apply(Box<Popped>),
    Idle,
    Ended(MediaError),
}

/// Bounded typed input. The mutex guards accounting, ownership transfer and indexes only: no callback and no payload drop run under it.
pub(crate) struct InputQueue {
    inner: Mutex<Inner>,
    limits: IngressLimits,
    wake: Notify,
    ended: watch::Sender<bool>,
}

/// Reserved bytes of data an executor-owned record keeps. Released when the record drops or is replaced.
pub(crate) struct Reserve {
    queue: Option<Arc<InputQueue>>,
    bytes: u64,
}

impl Reserve {
    pub fn none() -> Reserve {
        Reserve { queue: None, bytes: 0 }
    }
}

impl Drop for Reserve {
    fn drop(&mut self) {
        if let Some(queue) = &self.queue {
            queue.inner().reduce(self.bytes, 0);
        }
    }
}

/// One admitted entry slot, freed when the popped command is done.
pub(crate) struct EntryPlace(Option<Arc<InputQueue>>);

impl Drop for EntryPlace {
    fn drop(&mut self) {
        if let Some(queue) = &self.0 { queue.inner().reduce(0, 1); }
    }
}

enum Hold {
    /// A control: one admitted place, freed when the control ends.
    Control,
    /// A queued query: cancelling moves the entry and its reservation to the garbage list until the executor frees it.
    Query(u64),
}

/// The caller side of one admitted unit of work. A query keeps its reservation with the command and later with its running work, never here.
pub(crate) struct Admitted {
    queue: Arc<InputQueue>,
    hold: Hold,
}

impl Drop for Admitted {
    fn drop(&mut self) {
        let mut inner = self.queue.inner();
        match self.hold {
            Hold::Control => inner.reduce(0, 1),
            Hold::Query(seq) => inner.discard(seq),
        }
        drop(inner);
        self.queue.wake.notify_one();
    }
}

impl InputQueue {
    pub fn new(limits: IngressLimits) -> Arc<InputQueue> {
        Arc::new(InputQueue {
            inner: Mutex::new(Inner {
                seq: 0,
                epoch: 0,
                entries: BTreeMap::new(),
                folds: HashMap::new(),
                by_owner: HashMap::new(),
                owners: HashMap::new(),
                garbage: Vec::new(),
                counted: 0,
                bytes: 0,
                phase: Phase::Open,
            }),
            limits,
            wake: Notify::new(),
            ended: watch::channel(false).0,
        })
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn wake(&self) -> &Notify {
        &self.wake
    }

    pub fn ended(&self) -> watch::Receiver<bool> {
        self.ended.subscribe()
    }

    pub fn end(&self, error: MediaError) {
        let mut inner = self.inner();
        if matches!(inner.phase, Phase::Open) {
            inner.phase = Phase::Ended(error);
        }
        drop(inner);
        self.ended.send_replace(true);
        self.wake.notify_one();
    }

    pub fn end_error(&self) -> Option<MediaError> {
        match &self.inner().phase {
            Phase::Open => None,
            Phase::Ended(error) => Some(error.clone()),
        }
    }

    fn overloaded(what: &str) -> MediaError {
        MediaError::Overloaded { detail: format!("{what} limit reached") }
    }

    fn admit(&self, inner: &Inner, bytes: u64, entries: usize) -> Result<(), MediaError> {
        if let Phase::Ended(error) = &inner.phase {
            return Err(error.clone());
        }
        if inner.counted + entries > self.limits.entries as usize {
            return Err(Self::overloaded("entry"));
        }
        match inner.bytes.checked_add(bytes) {
            Some(total) if total <= self.limits.bytes => Ok(()),
            _ => Err(Self::overloaded("byte")),
        }
    }

    /// Registers a new owner together with its opening command. A refused command is dropped after the lock is released.
    pub fn open(&self, owner: u64, kind: OwnerKind, command: Declared) -> Result<(), MediaError> {
        let Some(bytes) = command.weight() else { return Err(Self::overloaded("byte")) };
        let mut inner = self.inner();
        if let Phase::Ended(error) = &inner.phase {
            let error = error.clone();
            drop(inner);
            drop(command);
            return Err(error);
        }
        if inner.owners.len() >= self.limits.owners as usize {
            drop(inner);
            drop(command);
            return Err(Self::overloaded("owner"));
        }
        if let Err(refused) = self.push(&mut inner, command, bytes) {
            let (error, rejected) = *refused;
            drop(inner);
            drop(rejected);
            return Err(error);
        }
        inner.owners.insert(owner, Owner { kind, is_closed: false });
        drop(inner);
        self.wake.notify_one();
        Ok(())
    }

    /// A refused command is dropped after the lock is released.
    pub fn submit(&self, command: Declared) -> Result<(), MediaError> {
        let Some(bytes) = command.weight() else { return Err(Self::overloaded("byte")) };
        let mut inner = self.inner();
        let refusal = match &inner.phase {
            Phase::Ended(error) => Some(error.clone()),
            Phase::Open => command.owner().filter(|owner| !inner.owners.get(owner).is_some_and(|record| !record.is_closed)).map(|_| MediaError::Closed),
        };
        if let Some(error) = refusal {
            drop(inner);
            drop(command);
            return Err(error);
        }
        if let Err(refused) = self.push(&mut inner, command, bytes) {
            let (error, rejected) = *refused;
            drop(inner);
            drop(rejected);
            return Err(error);
        }
        drop(inner);
        self.wake.notify_one();
        Ok(())
    }

    pub fn submit_audio(&self, commands: Vec<Declared>) -> Result<(), MediaError> {
        let mut inner = self.inner();
        if let Phase::Ended(error) = &inner.phase { return Err(error.clone()); }
        for command in commands {
            let seq = inner.seq;
            inner.seq += 1;
            inner.entries.insert(seq, Entry { command, bytes: 0, fold: None, owner: None });
            inner.epoch += 1;
        }
        drop(inner);
        self.wake.notify_one();
        Ok(())
    }

    fn push(&self, inner: &mut Inner, command: Declared, bytes: u64) -> Result<(), Box<(MediaError, Declared)>> {
        let fold = command.fold();
        if let Some(fold) = &fold {
            let epoch = inner.epoch;
            let queued = inner.folds.get(fold).filter(|slot| slot.epoch == epoch).map(|slot| slot.seq);
            if let Some(seq) = queued.filter(|seq| inner.entries.contains_key(seq)) {
                return self.replace(inner, seq, command, bytes);
            }
        }
        if let Err(error) = self.admit(inner, bytes, 1) {
            return Err(Box::new((error, command)));
        }
        let seq = inner.seq;
        inner.seq += 1;
        let owner = command.owner();
        let barrier = command.is_barrier();
        inner.counted += 1;
        inner.bytes += bytes;
        if let Some(owner) = owner {
            inner.by_owner.entry(owner).or_default().push(seq);
        }
        if let Some(fold) = &fold {
            let epoch = inner.epoch;
            inner.folds.insert(fold.clone(), FoldSlot { seq, epoch });
        }
        inner.entries.insert(seq, Entry { command, bytes, fold, owner });
        if barrier {
            inner.epoch += 1;
        }
        Ok(())
    }

    fn replace(&self, inner: &mut Inner, seq: u64, command: Declared, incoming: u64) -> Result<(), Box<(MediaError, Declared)>> {
        if let Err(error) = self.admit(inner, incoming, 0) {
            return Err(Box::new((error, command)));
        }
        let Some(entry) = inner.entries.get_mut(&seq) else {
            return Err(Box::new((MediaError::Unexpected { detail: "a fold slot points at a missing entry".to_string() }, command)));
        };
        let previous = entry.bytes;
        let (discarded, merge) = entry.command.absorb(command);
        let Merge::Replaced = merge else { return Ok(()) };
        entry.bytes = incoming;
        let held = if discarded.is_some() { previous } else { 0 };
        inner.bytes = inner.bytes.saturating_sub(previous).saturating_add(incoming).saturating_add(held);
        if let Some(command) = discarded {
            inner.garbage.push(Garbage { command, bytes: previous, counted: false });
        }
        Ok(())
    }

    /// Closes the owner at once and without a free queue place: it only marks the owner, moves its entries to the garbage list (a `PlaceSeek` stays queued in arrival order) and wakes the executor.
    pub fn release(&self, owner: u64) {
        let mut inner = self.inner();
        let Some(record) = inner.owners.get_mut(&owner).filter(|record| !record.is_closed) else { return };
        record.is_closed = true;
        let kind = record.kind;
        for seq in inner.by_owner.remove(&owner).unwrap_or_default() {
            match inner.entries.get_mut(&seq) {
                Some(entry) if matches!(entry.command, Declared::PlaceSeek { .. }) => entry.owner = None,
                _ => inner.discard(seq),
            }
        }
        let seq = inner.seq;
        inner.seq += 1;
        inner.entries.insert(seq, Entry { command: Declared::Release { owner, kind }, bytes: 0, fold: None, owner: None });
        inner.epoch += 1;
        drop(inner);
        self.wake.notify_one();
    }

    pub fn is_live(&self, owner: u64) -> bool {
        self.inner().owners.get(&owner).is_some_and(|owner| !owner.is_closed)
    }

    pub fn finish_release(&self, owner: u64) {
        let mut inner = self.inner();
        inner.owners.remove(&owner);
        inner.by_owner.remove(&owner);
        #[cfg(test)]
        crate::test_signal::fire();
    }

    /// Takes the garbage so the caller of this method frees it outside the lock, then reports it with `freed`.
    pub fn take_garbage(&self) -> Vec<Garbage> {
        std::mem::take(&mut self.inner().garbage)
    }

    pub fn freed(&self, garbage: &[(u64, bool)]) {
        let bytes = garbage.iter().map(|(bytes, _)| *bytes).sum();
        let counted = garbage.iter().filter(|(_, counted)| *counted).count();
        self.inner().reduce(bytes, counted);
    }

    pub fn next(self: &Arc<Self>) -> Upcoming {
        let mut inner = self.inner();
        if let Phase::Ended(error) = &inner.phase {
            return Upcoming::Ended(error.clone());
        }
        let Some(seq) = inner.entries.keys().next().copied() else { return Upcoming::Idle };
        let Some(entry) = inner.entries.remove(&seq) else { return Upcoming::Idle };
        if let Some(fold) = &entry.fold {
            if inner.folds.get(fold).is_some_and(|slot| slot.seq == seq) {
                inner.folds.remove(fold);
            }
        }
        if let Some(owner) = entry.owner {
            if let Some(seqs) = inner.by_owner.get_mut(&owner) {
                seqs.retain(|queued| *queued != seq);
                if seqs.is_empty() {
                    inner.by_owner.remove(&owner);
                }
            }
        }
        if let Declared::Release { owner, kind } = entry.command {
            return Upcoming::Release(owner, kind);
        }
        let counted = !matches!(entry.command, Declared::Audio(_));
        Upcoming::Apply(Box::new(Popped { command: entry.command, reserve: Reserve { queue: Some(self.clone()), bytes: entry.bytes }, owner: entry.owner, slot: EntryPlace(counted.then(|| self.clone())) }))
    }

    /// Takes every queued entry out after the end so the executor can answer them, and writes their accounting off.
    pub fn drain(&self) -> Vec<Declared> {
        let mut inner = self.inner();
        let entries = std::mem::take(&mut inner.entries);
        let garbage = std::mem::take(&mut inner.garbage);
        inner.folds.clear();
        inner.by_owner.clear();
        inner.owners.clear();
        let bytes = entries.values().map(|entry| entry.bytes).chain(garbage.iter().map(|garbage| garbage.bytes)).sum();
        let queued = entries.values().filter(|entry| !matches!(entry.command, Declared::Release { .. } | Declared::Audio(_))).count();
        inner.reduce(bytes, queued + garbage.iter().filter(|garbage| garbage.counted).count());
        drop(inner);
        entries.into_values().map(|entry| entry.command).chain(garbage.into_iter().map(|garbage| garbage.command)).collect()
    }

    /// Admits one waiting-or-running unit of work (a control) without queuing a command.
    pub fn hold(self: &Arc<Self>) -> Result<Admitted, MediaError> {
        let mut inner = self.inner();
        self.admit(&inner, 0, 1)?;
        inner.counted += 1;
        Ok(Admitted { queue: self.clone(), hold: Hold::Control })
    }

    /// A query: its parameters are reserved and it is ordered behind everything accepted so far.
    pub fn query(self: &Arc<Self>, query: Query) -> Result<(Admitted, oneshot::Receiver<Answer>), MediaError> {
        let (reply, answer) = oneshot::channel();
        let command = Declared::Query { query, reply };
        let bytes = command.weight().ok_or_else(|| Self::overloaded("byte"))?;
        let mut inner = self.inner();
        self.admit(&inner, bytes, 1)?;
        let seq = inner.seq;
        inner.seq += 1;
        inner.counted += 1;
        inner.bytes += bytes;
        inner.entries.insert(seq, Entry { command, bytes, fold: None, owner: None });
        inner.epoch += 1;
        drop(inner);
        self.wake.notify_one();
        Ok((Admitted { queue: self.clone(), hold: Hold::Query(seq) }, answer))
    }

    #[cfg(test)]
    pub fn is_unlocked(&self) -> bool {
        self.inner.try_lock().is_ok()
    }

    #[cfg(test)]
    pub fn audit(&self) -> Audit {
        let inner = self.inner();
        Audit {
            counted: inner.counted,
            bytes: inner.bytes,
            entries: inner.entries.values().filter(|entry| !matches!(entry.command, Declared::Release { .. })).count(),
            folds: inner.folds.len(),
            by_owner: inner.by_owner.len(),
            owners: inner.owners.len(),
            releases: inner.entries.values().filter(|entry| matches!(entry.command, Declared::Release { .. })).count(),
            garbage: inner.garbage.len(),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Audit {
    pub counted: usize,
    pub bytes: u64,
    pub entries: usize,
    pub folds: usize,
    pub by_owner: usize,
    pub owners: usize,
    pub releases: usize,
    pub garbage: usize,
}

impl Garbage {
    pub fn account(&self) -> (u64, bool) {
        (self.bytes, self.counted)
    }
}
