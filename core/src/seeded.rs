const DEFAULT_SEED: u64 = 0x4d4c_2026;
const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

pub fn seed() -> u64 {
    std::env::var("ML_SEED").ok().and_then(|text| text.trim().parse().ok()).unwrap_or(DEFAULT_SEED)
}

pub struct Dice {
    state: u64,
    seed: u64,
    root: u64,
    test: &'static str,
    steps: u64,
}

fn mix(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(GOLDEN);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}

impl Dice {
    pub fn new(test: &'static str) -> Dice {
        Dice::from_seed(test, seed())
    }

    pub fn from_seed(test: &'static str, seed: u64) -> Dice {
        Dice { state: seed, seed, root: seed, test, steps: 0 }
    }

    pub fn fork(&self, lane: u64) -> Dice {
        let mut forked = Dice::from_seed(self.test, mix(self.seed ^ mix(lane.wrapping_add(1))));
        forked.root = self.root;
        forked
    }

    pub fn next_u64(&mut self) -> u64 {
        self.steps += 1;
        self.state = self.state.wrapping_add(GOLDEN);
        mix(self.state)
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound.max(1)
    }

    pub fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    pub fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    pub fn percent(&mut self, chance: u64) -> bool {
        self.below(100) < chance
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u64() as u8).collect()
    }
}

impl Drop for Dice {
    fn drop(&mut self) {
        if std::thread::panicking() && self.seed == self.root {
            eprintln!(
                "seeded test {} failed after {} draws. Reproduce: ML_SEED={} cargo test --manifest-path core/Cargo.toml --lib {}",
                self.test,
                self.steps,
                self.root,
                self.test
            );
        }
    }
}
