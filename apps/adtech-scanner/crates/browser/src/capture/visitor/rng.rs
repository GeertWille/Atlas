//! The random source of a scripted visit. Seeded, so that a scan can be
//! repeated action for action; SplitMix64, because nothing here needs more.

#[derive(Debug, Clone)]
pub(crate) struct Rng {
    state: u64,
}

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// An independent stream for one part of the visit, so that what one page
    /// draws does not shift what the next page draws.
    pub(crate) fn fork(&self, stream: u64) -> Self {
        let mut forked = Self::new(self.state ^ stream.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        forked.next_u64();
        forked
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub(crate) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[low, high)`.
    pub(crate) fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    /// Uniform over `low..=high`.
    pub(crate) fn whole(&mut self, low: u32, high: u32) -> u32 {
        let span = f64::from(high - low + 1);
        low + ((self.unit() * span) as u32).min(high - low)
    }

    pub(crate) fn chance(&mut self, probability: f64) -> bool {
        self.unit() < probability
    }

    /// Index into `weights`, each chosen in proportion to its weight. `None`
    /// when nothing has a positive weight.
    pub(crate) fn weighted(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().filter(|weight| **weight > 0.0).sum();
        if total <= 0.0 {
            return None;
        }
        let mut remaining = self.unit() * total;
        let mut last_positive = None;
        for (index, weight) in weights.iter().enumerate() {
            if *weight <= 0.0 {
                continue;
            }
            last_positive = Some(index);
            if remaining < *weight {
                return last_positive;
            }
            remaining -= weight;
        }
        // Rounding can leave a sliver past the last weight.
        last_positive
    }

    /// Log-normally distributed around `median`, never above `cap`.
    pub(crate) fn log_normal(&mut self, median: f64, sigma: f64, cap: f64) -> f64 {
        // Box-Muller; `1 - unit` keeps the logarithm's argument above zero.
        let radius = (-2.0 * (1.0 - self.unit()).ln()).sqrt();
        let normal = radius * (std::f64::consts::TAU * self.unit()).cos();
        (median * (sigma * normal).exp()).min(cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_numbers_and_another_seed_others() {
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            (0..8).map(|_| rng.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(draw(7), draw(7));
        assert_ne!(draw(7), draw(8));
        // Reference values of SplitMix64 for seed 0.
        assert_eq!(draw(0)[0], 0xe220_a839_7b1d_cdaf);
        assert_eq!(draw(0)[1], 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn draws_stay_inside_their_ranges() {
        let mut rng = Rng::new(42);
        for _ in 0..10_000 {
            let unit = rng.unit();
            assert!((0.0..1.0).contains(&unit));
            let between = rng.between(-3.0, 9.0);
            assert!((-3.0..9.0).contains(&between));
            assert!((3..=9).contains(&rng.whole(3, 9)));
            assert!(rng.log_normal(1_200.0, 0.6, 6_000.0) <= 6_000.0);
        }
        assert_eq!(rng.whole(5, 5), 5);
    }

    #[test]
    fn whole_numbers_cover_both_ends() {
        let mut rng = Rng::new(1);
        let drawn: std::collections::BTreeSet<u32> = (0..500).map(|_| rng.whole(1, 4)).collect();
        assert_eq!(drawn.into_iter().collect::<Vec<_>>(), [1, 2, 3, 4]);
    }

    #[test]
    fn weighted_choice_follows_the_weights_and_skips_zero() {
        let mut rng = Rng::new(3);
        let mut counts = [0usize; 4];
        for _ in 0..4_000 {
            counts[rng.weighted(&[1.0, 0.0, 3.0, -2.0]).unwrap()] += 1;
        }
        assert_eq!((counts[1], counts[3]), (0, 0));
        let share = counts[2] as f64 / 4_000.0;
        assert!((0.70..0.80).contains(&share), "{share}");
        assert_eq!(rng.weighted(&[]), None);
        assert_eq!(rng.weighted(&[0.0, 0.0]), None);
    }

    #[test]
    fn the_median_of_log_normal_draws_is_the_median_asked_for() {
        let mut rng = Rng::new(11);
        let mut draws: Vec<f64> = (0..2_001)
            .map(|_| rng.log_normal(1_200.0, 0.6, f64::MAX))
            .collect();
        draws.sort_by(f64::total_cmp);
        assert!(
            (1_050.0..1_350.0).contains(&draws[1_000]),
            "{}",
            draws[1_000]
        );
    }

    #[test]
    fn a_fork_is_decided_by_its_stream_and_by_where_the_parent_is() {
        let rng = Rng::new(99);
        let first = rng.fork(1).unit();
        let mut advanced = rng.clone();
        advanced.unit();
        assert_eq!(rng.fork(1).unit(), first);
        assert_ne!(rng.fork(2).unit(), first);
        assert_ne!(advanced.fork(1).unit(), first);
    }
}
