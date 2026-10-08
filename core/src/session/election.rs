use super::suspensions::{Reason, SuspensionError, Suspensions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeMode {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Play,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub group: String,
    pub priority: u32,
    pub resume: ResumeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Ring,
    Ambient,
    Hero,
    Overlay,
    System,
    Exclusive,
}

impl Context {
    pub fn policy(self) -> Policy {
        let (group, priority, resume) = match self {
            Context::Ring => ("ring", 300, ResumeMode::Automatic),
            Context::Ambient => ("ambient", 325, ResumeMode::Automatic),
            Context::Hero => ("hero", 400, ResumeMode::Manual),
            Context::Overlay => ("overlay", 450, ResumeMode::Automatic),
            Context::System => ("system", 475, ResumeMode::Automatic),
            Context::Exclusive => ("exclusive", 500, ResumeMode::Automatic),
        };
        Policy { group: group.to_string(), priority, resume }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Play,
    Pause,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseOrder {
    pub lease: u64,
    pub order: Order,
}

#[derive(Debug, Clone)]
struct Session {
    key: String,
    lease: u64,
    policy: Policy,
    intent: Intent,
    sequence: u64,
    is_playing: bool,
}

#[derive(Debug, Clone)]
pub struct Election {
    is_foreground: bool,
    active: Option<u64>,
    sessions: Vec<Session>,
    suspensions: Suspensions,
    next_lease: u64,
    next_sequence: u64,
}

impl Default for Election {
    fn default() -> Self {
        Self::new()
    }
}

impl Election {
    pub fn new() -> Self {
        Self { is_foreground: true, active: None, sessions: Vec::new(), suspensions: Suspensions::default(), next_lease: 1, next_sequence: 1 }
    }

    pub fn register(&mut self, key: &str, policy: Policy) -> u64 {
        let lease = self.next_lease;
        self.next_lease += 1;
        self.sessions.push(Session { key: key.to_string(), lease, policy, intent: Intent::Idle, sequence: 0, is_playing: false });
        lease
    }

    pub fn unregister(&mut self, key: &str, lease: u64) -> Vec<LeaseOrder> {
        let mut out = Vec::new();
        let Some(index) = self.sessions.iter().position(|session| session.lease == lease && session.key == key) else {
            return out;
        };
        self.sessions.remove(index);
        if self.active == Some(lease) {
            self.active = None;
            out.push(LeaseOrder { lease, order: Order::Pause });
        }
        self.resolve(true, &mut out);
        out
    }

    pub fn set_intent(&mut self, key: &str, lease: u64, intent: Intent) -> Vec<LeaseOrder> {
        let mut out = Vec::new();
        let sequence = self.next_sequence;
        let Some(session) = self.sessions.iter_mut().find(|session| session.lease == lease && session.key == key) else {
            return out;
        };
        if session.intent == intent {
            return out;
        }
        session.intent = intent;
        if intent == Intent::Play {
            session.sequence = sequence;
            self.next_sequence += 1;
        }
        self.resolve(intent == Intent::Play, &mut out);
        out
    }

    pub fn set_foreground(&mut self, is_foreground: bool) -> Vec<LeaseOrder> {
        let mut out = Vec::new();
        if self.is_foreground == is_foreground {
            return out;
        }
        self.is_foreground = is_foreground;
        self.resolve(false, &mut out);
        out
    }

    pub fn set_suspension(&mut self, reason: Reason, is_active: bool, allow_groups: Option<&[&str]>) -> Result<Vec<LeaseOrder>, SuspensionError> {
        let mut out = Vec::new();
        if self.suspensions.set(reason, is_active, allow_groups)? {
            self.resolve(true, &mut out);
        }
        Ok(out)
    }

    pub fn stop_all(&mut self) -> Vec<LeaseOrder> {
        self.stop(|_| true)
    }

    pub fn stop_manual(&mut self) -> Vec<LeaseOrder> {
        let mut out = self.stop(|session| session.policy.resume == ResumeMode::Manual);
        if !out.is_empty() {
            self.resolve(false, &mut out);
        }
        out
    }

    pub fn active(&self) -> Option<u64> {
        self.active
    }

    pub fn is_active(&self, key: &str, lease: u64) -> bool {
        self.active == Some(lease) && self.sessions.iter().any(|session| session.lease == lease && session.key == key)
    }

    pub fn intent(&self, lease: u64) -> Option<Intent> {
        self.sessions.iter().find(|session| session.lease == lease).map(|session| session.intent)
    }

    pub fn is_eligible(&self, lease: u64) -> bool {
        self.sessions.iter().any(|session| session.lease == lease && self.is_session_eligible(session))
    }

    fn is_session_eligible(&self, session: &Session) -> bool {
        self.is_foreground && session.intent == Intent::Play && self.suspensions.allows(&session.policy.group)
    }

    fn stop(&mut self, matches: impl Fn(&Session) -> bool) -> Vec<LeaseOrder> {
        let mut out = Vec::new();
        let mut affected: Vec<u64> =
            self.sessions.iter().filter(|session| session.intent == Intent::Play && matches(session)).map(|session| session.lease).collect();
        if affected.is_empty() {
            return out;
        }
        affected.sort_unstable();
        let stopped_active = self.active.filter(|lease| affected.contains(lease));
        for session in self.sessions.iter_mut().filter(|session| affected.contains(&session.lease)) {
            session.intent = Intent::Idle;
            session.is_playing = false;
        }
        if let Some(lease) = stopped_active {
            self.active = None;
            out.push(LeaseOrder { lease, order: Order::Pause });
            out.push(LeaseOrder { lease, order: Order::Idle });
        }
        out.extend(affected.into_iter().filter(|lease| Some(*lease) != stopped_active).map(|lease| LeaseOrder { lease, order: Order::Idle }));
        out
    }

    fn winner(&self) -> Option<u64> {
        self.sessions
            .iter()
            .filter(|session| self.is_session_eligible(session))
            .min_by(|a, b| {
                b.policy
                    .priority
                    .cmp(&a.policy.priority)
                    .then(b.sequence.cmp(&a.sequence))
                    .then(a.key.cmp(&b.key))
                    .then(a.lease.cmp(&b.lease))
            })
            .map(|session| session.lease)
    }

    fn resolve(&mut self, manual_loss_becomes_idle: bool, out: &mut Vec<LeaseOrder>) {
        if self.winner() == self.active {
            return;
        }
        if let Some(lease) = self.active.take() {
            if let Some(session) = self.sessions.iter_mut().find(|session| session.lease == lease) {
                session.is_playing = false;
                out.push(LeaseOrder { lease, order: Order::Pause });
                if session.policy.resume == ResumeMode::Manual && manual_loss_becomes_idle {
                    session.intent = Intent::Idle;
                    out.push(LeaseOrder { lease, order: Order::Idle });
                }
            }
        }
        let Some(next) = self.winner() else {
            return;
        };
        if let Some(session) = self.sessions.iter_mut().find(|session| session.lease == next) {
            session.is_playing = true;
        }
        self.active = Some(next);
        out.push(LeaseOrder { lease: next, order: Order::Play });
    }
}
