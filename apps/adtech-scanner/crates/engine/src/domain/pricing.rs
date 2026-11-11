//! Token usage, model prices and the per-scan LLM budget.

use super::DomainError;

pub const MAX_BUDGET_USD: f64 = 50.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// Uncached input tokens.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn prompt_tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, other: TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

const HAIKU_LONG_PROMPT_TOKENS: u64 = 100_000;

const HAIKU_5_5: Rates = Rates {
    input: 0.10,
    output: 0.50,
    cache_read: 0.01,
    cache_write: 0.125,
};
const HAIKU_5_5_LONG_PROMPT: Rates = Rates {
    input: 0.50,
    output: 2.50,
    cache_read: 0.05,
    cache_write: 0.625,
};
const SONNET_5_5: Rates = Rates {
    input: 2.0,
    output: 10.0,
    cache_read: 0.10,
    cache_write: 2.50,
};
/// Applied to models missing from the table so an unknown model can never
/// under-count against the budget.
const CONSERVATIVE: Rates = Rates {
    input: 15.0,
    output: 75.0,
    cache_read: 15.0,
    cache_write: 18.75,
};

fn rates_for(model: &str, usage: TokenUsage) -> Option<Rates> {
    match model {
        "claude-haiku-5-5" if usage.prompt_tokens() > HAIKU_LONG_PROMPT_TOKENS => {
            Some(HAIKU_5_5_LONG_PROMPT)
        }
        "claude-haiku-5-5" => Some(HAIKU_5_5),
        "claude-sonnet-5-5" => Some(SONNET_5_5),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CallCost {
    pub usd: f64,
    /// False when the model is not in the price table and the conservative
    /// fallback price was used.
    pub known_model: bool,
}

pub fn cost_of(model: &str, usage: TokenUsage) -> CallCost {
    let rates = rates_for(model, usage);
    let applied = rates.unwrap_or(CONSERVATIVE);
    let per_million = usage.input as f64 * applied.input
        + usage.output as f64 * applied.output
        + usage.cache_read as f64 * applied.cache_read
        + usage.cache_write as f64 * applied.cache_write;
    CallCost {
        usd: per_million / 1_000_000.0,
        known_model: rates.is_some(),
    }
}

/// Rough token count for text we are about to send (about 3.5 chars per token
/// for minified JavaScript and JSON; rounded up to stay on the safe side).
pub fn estimate_tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(3)
}

/// Hard cap on LLM spend for one scan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    limit_usd: f64,
    spent_usd: f64,
}

impl Budget {
    pub fn new(limit_usd: f64) -> Result<Self, DomainError> {
        if !limit_usd.is_finite() || !(0.0..=MAX_BUDGET_USD).contains(&limit_usd) {
            return Err(DomainError::BudgetOutOfRange { max: MAX_BUDGET_USD });
        }
        Ok(Self {
            limit_usd,
            spent_usd: 0.0,
        })
    }

    pub fn limit_usd(&self) -> f64 {
        self.limit_usd
    }

    pub fn spent_usd(&self) -> f64 {
        self.spent_usd
    }

    pub fn remaining_usd(&self) -> f64 {
        (self.limit_usd - self.spent_usd).max(0.0)
    }

    pub fn charge(&mut self, usd: f64) {
        self.spent_usd += usd.max(0.0);
    }

    /// Whether a call projected to cost `projected_usd` still fits.
    pub fn can_afford(&self, projected_usd: f64) -> bool {
        self.spent_usd + projected_usd <= self.limit_usd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenUsage {
        TokenUsage {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    #[test]
    fn sonnet_cost_includes_cache_reads_and_writes() {
        let cost = cost_of("claude-sonnet-5-5", usage(1_000_000, 100_000, 2_000_000, 400_000));
        assert!(cost.known_model);
        assert!((cost.usd - (2.0 + 1.0 + 0.2 + 1.0)).abs() < 1e-9);
    }

    #[test]
    fn haiku_switches_rate_card_above_100k_prompt_tokens() {
        let short = cost_of("claude-haiku-5-5", usage(100_000, 1_000, 0, 0));
        assert!((short.usd - (0.01 + 0.0005)).abs() < 1e-9);
        let long = cost_of("claude-haiku-5-5", usage(90_000, 1_000, 20_000, 0));
        assert!((long.usd - (0.045 + 0.0025 + 0.001)).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_uses_conservative_price() {
        let cost = cost_of("some-future-model", usage(1_000_000, 0, 0, 0));
        assert!(!cost.known_model);
        assert!(cost.usd >= cost_of("claude-sonnet-5-5", usage(1_000_000, 0, 0, 0)).usd);
    }

    #[test]
    fn budget_rejects_invalid_limits_and_tracks_spend() {
        assert!(Budget::new(-0.01).is_err());
        assert!(Budget::new(f64::NAN).is_err());
        assert!(Budget::new(MAX_BUDGET_USD + 1.0).is_err());
        let mut budget = Budget::new(0.5).unwrap();
        assert!(budget.can_afford(0.5));
        budget.charge(0.3);
        assert!(budget.can_afford(0.2));
        assert!(!budget.can_afford(0.21));
        assert!((budget.remaining_usd() - 0.2).abs() < 1e-9);
    }
}
