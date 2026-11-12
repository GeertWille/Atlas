//! Token usage, model prices and the per-scan LLM budget.

use super::DomainError;

const MAX_BUDGET_USD: f64 = 50.0;

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
}

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

/// Prompts longer than this are billed at a model's long-prompt rates, where
/// it has them.
const LONG_PROMPT_TOKENS: u64 = 100_000;

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
/// The Jev System One release the triage thresholds are written against. The
/// versioned id is used instead of the `jev-latest` alias, which moves between
/// releases and would change answers without a change here.
pub const JEV_SYSTEM_ONE: &str = "jev-1.14.0";

/// Jev bills input only; one price for every published release and alias.
const JEV: Rates = Rates {
    input: 0.042,
    output: 0.0,
    cache_read: 0.0,
    cache_write: 0.0,
};
/// Applied to models missing from the table so an unknown model can never
/// under-count against the budget.
const CONSERVATIVE: Rates = Rates {
    input: 15.0,
    output: 75.0,
    cache_read: 15.0,
    cache_write: 18.75,
};

struct Price {
    model: &'static str,
    rates: Rates,
    long_prompt: Option<Rates>,
}

/// Every model with a price. The frontend estimates a scan's cost from
/// `src/shared/domain/model-prices.json`, which a test below writes from
/// this table.
const PRICE_TABLE: &[Price] = &[
    Price {
        model: "claude-haiku-5-5",
        rates: HAIKU_5_5,
        long_prompt: Some(HAIKU_5_5_LONG_PROMPT),
    },
    Price {
        model: "claude-sonnet-5-5",
        rates: SONNET_5_5,
        long_prompt: None,
    },
    Price {
        model: JEV_SYSTEM_ONE,
        rates: JEV,
        long_prompt: None,
    },
    Price {
        model: "jev-1.13.0",
        rates: JEV,
        long_prompt: None,
    },
    Price {
        model: "jev-latest",
        rates: JEV,
        long_prompt: None,
    },
    Price {
        model: "jev-preview",
        rates: JEV,
        long_prompt: None,
    },
];

fn rates_for(model: &str, usage: TokenUsage) -> Option<Rates> {
    let price = PRICE_TABLE.iter().find(|price| price.model == model)?;
    Some(match price.long_prompt {
        Some(long_prompt) if usage.prompt_tokens() > LONG_PROMPT_TOKENS => long_prompt,
        _ => price.rates,
    })
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

/// How many output tokens `remaining_usd` still pays for after the prompt
/// described by `prompt` (its `output` is ignored) has been billed.
pub fn affordable_output_tokens(model: &str, prompt: TokenUsage, remaining_usd: f64) -> u64 {
    let prompt = TokenUsage {
        output: 0,
        ..prompt
    };
    let rates = rates_for(model, prompt).unwrap_or(CONSERVATIVE);
    let left_for_output = remaining_usd - cost_of(model, prompt).usd;
    if left_for_output <= 0.0 {
        return 0;
    }
    (left_for_output * 1_000_000.0 / rates.output) as u64
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
            return Err(DomainError::BudgetOutOfRange {
                max: MAX_BUDGET_USD,
            });
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
        let cost = cost_of(
            "claude-sonnet-5-5",
            usage(1_000_000, 100_000, 2_000_000, 400_000),
        );
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
    fn affordable_output_is_what_remains_after_the_prompt() {
        // $0.05 left, 10K uncached prompt tokens cost $0.02, $0.03 buys 3000 output tokens.
        assert_eq!(
            affordable_output_tokens("claude-sonnet-5-5", usage(10_000, 999, 0, 0), 0.05),
            3_000
        );
        assert_eq!(
            affordable_output_tokens("claude-sonnet-5-5", usage(10_000, 0, 0, 0), 0.02),
            0
        );
        assert_eq!(
            affordable_output_tokens("claude-sonnet-5-5", usage(10_000, 0, 0, 0), 0.0),
            0
        );
        assert!(affordable_output_tokens("unknown", usage(0, 0, 0, 0), 0.75) == 10_000);
    }

    #[test]
    fn jev_bills_input_only_under_its_versioned_ids_and_aliases() {
        for model in [JEV_SYSTEM_ONE, "jev-1.13.0", "jev-latest", "jev-preview"] {
            let cost = cost_of(model, usage(1_000_000, 500_000, 0, 0));
            assert!(cost.known_model, "{model}");
            assert!((cost.usd - 0.042).abs() < 1e-12, "{model}: {}", cost.usd);
        }
        assert!(!cost_of("jev-9.0.0", usage(1, 0, 0, 0)).known_model);
    }

    /// The frontend's copy of the prices is this file and nothing else. When
    /// the table changed, the file is written again and the test fails once,
    /// so that the change is seen and committed.
    #[test]
    fn the_frontend_price_file_is_this_table() {
        use serde_json::{Map, Value, json};

        const FILE: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/shared/domain/model-prices.json"
        );
        let shown = |rates: Rates| json!({ "input": rates.input, "output": rates.output });
        let models: Map<String, Value> = PRICE_TABLE
            .iter()
            .map(|price| {
                let mut entry = shown(price.rates);
                entry["longPrompt"] = price.long_prompt.map_or(Value::Null, shown);
                (price.model.to_string(), entry)
            })
            .collect();
        // What this table charges for a few calls, for the frontend's own
        // arithmetic to be checked against: at and just above the long-prompt
        // limit, and for a model without a price.
        let checks: Vec<Value> = PRICE_TABLE
            .iter()
            .map(|price| price.model)
            .chain(["a-model-without-a-price"])
            .flat_map(|model| {
                [LONG_PROMPT_TOKENS, LONG_PROMPT_TOKENS + 1, 1_000_000].map(move |input| {
                    let cost = cost_of(model, usage(input, 7_000, 0, 0));
                    json!({
                        "model": model,
                        "input": input,
                        "output": 7_000,
                        "usd": cost.usd,
                        "knownModel": cost.known_model,
                    })
                })
            })
            .collect();
        let document = json!({
            "jevModel": JEV_SYSTEM_ONE,
            "longPromptTokens": LONG_PROMPT_TOKENS,
            "models": models,
            "unknownModel": shown(CONSERVATIVE),
            "checks": checks,
        });
        let expected = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());

        let on_disk = std::fs::read_to_string(FILE).unwrap_or_default();
        if on_disk != expected {
            std::fs::write(FILE, &expected).unwrap();
            panic!("{FILE} did not match the price table and was written again; commit it");
        }
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
