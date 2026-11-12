//! Validates the adapter params of the page's ad units against what the
//! adapter itself requires. Prebid drops a bid with invalid params before any
//! request is made, so such a bidder is silently absent from the auction.

use std::collections::{BTreeMap, BTreeSet};

use adscan_model::{DiagnosticArea, DiagnosticCheck, DiagnosticStatus};
use serde_json::Value;

use super::adapters::{AdapterSpec, ParamType};
use super::{check, codes, excerpt};

/// One bid of an ad unit as the page configured it.
#[derive(Debug, Clone, Copy)]
pub struct BidConfig<'a> {
    pub ad_unit: &'a str,
    pub sizes: &'a [String],
    pub params: &'a Value,
}

/// Sharing one placement id is only flagged from this many ad units on.
const MIN_AD_UNITS_FOR_SHARED_ID: usize = 3;
const MAX_VALUE_CHARS: usize = 40;

/// `config_has` tells whether the page's Prebid config sets a key.
pub fn check_params(
    adapter: &AdapterSpec,
    bids: &[BidConfig],
    config_has: impl Fn(&str) -> bool,
) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();
    let waived = adapter.waived_by_config.is_some_and(config_has);
    if !waived {
        checks.extend(missing_params(adapter, bids));
    }
    checks.extend(wrong_types(adapter, bids));
    checks.extend(test_ids(adapter, bids));
    checks.extend(shared_placement_id(adapter, bids));
    if checks.is_empty() {
        let summary = if adapter.required.is_empty() && adapter.types.is_empty() {
            "This adapter has no required params".to_string()
        } else {
            format!("Required params are set on {}", all_ad_units(bids.len()))
        };
        checks.push(check(
            DiagnosticArea::Params,
            DiagnosticStatus::Pass,
            codes::PARAMS_OK,
            summary,
        ));
    }
    checks
}

fn missing_params(adapter: &AdapterSpec, bids: &[BidConfig]) -> Vec<DiagnosticCheck> {
    if adapter.required.is_empty() {
        return Vec::new();
    }
    // What is missing -> the ad units it is missing on.
    let mut by_missing: BTreeMap<Vec<&str>, Vec<&str>> = BTreeMap::new();
    for bid in bids {
        let missing = adapter
            .required
            .iter()
            .map(|alternative| missing_in(alternative, bid.params))
            .min_by_key(Vec::len)
            .unwrap_or_default();
        if !missing.is_empty() {
            by_missing.entry(missing).or_default().push(bid.ad_unit);
        }
    }
    by_missing
        .into_iter()
        .map(|(missing, ad_units)| {
            let mut summary = format!(
                "{} missing on {}",
                code_list(&missing),
                share_of_ad_units(ad_units.len(), bids.len())
            );
            if adapter.required.len() > 1 {
                summary.push_str(&format!(
                    " (the adapter needs {})",
                    describe_alternatives(adapter)
                ));
            }
            let mut check = check(
                DiagnosticArea::Params,
                DiagnosticStatus::Problem,
                codes::PARAMS_MISSING,
                summary,
            );
            check.ad_units = unique(&ad_units);
            check
        })
        .collect()
}

/// The first key of every param of the alternative that is not set.
fn missing_in<'a>(alternative: &[&'a [&'a str]], params: &Value) -> Vec<&'a str> {
    alternative
        .iter()
        .filter(|keys| !keys.iter().any(|key| is_truthy(params.get(key))))
        .map(|keys| keys[0])
        .collect()
}

fn describe_alternatives(adapter: &AdapterSpec) -> String {
    adapter
        .required
        .iter()
        .map(|alternative| {
            alternative
                .iter()
                .map(|keys| format!("`{}`", keys[0]))
                .collect::<Vec<_>>()
                .join(" with ")
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

fn wrong_types(adapter: &AdapterSpec, bids: &[BidConfig]) -> Vec<DiagnosticCheck> {
    adapter
        .types
        .iter()
        .filter_map(|(key, expected)| {
            let offending: Vec<&BidConfig> = bids
                .iter()
                .filter(|bid| {
                    bid.params
                        .get(key)
                        .is_some_and(|value| !value.is_null() && !fits(value, *expected))
                })
                .collect();
            let first = offending.first()?;
            let found = &first.params[*key];
            let mut check = check(
                DiagnosticArea::Params,
                DiagnosticStatus::Problem,
                codes::PARAMS_WRONG_TYPE,
                format!(
                    "`{key}` must be {}, found {} on {}",
                    describe_type(*expected),
                    describe_value(found),
                    share_of_ad_units(offending.len(), bids.len())
                ),
            );
            check.excerpt = Some(excerpt(&format!("{key}: {found}")));
            check.ad_units = unique(&offending.iter().map(|bid| bid.ad_unit).collect::<Vec<_>>());
            Some(check)
        })
        .collect()
}

fn fits(value: &Value, expected: ParamType) -> bool {
    match expected {
        ParamType::Text | ParamType::OptionalText => value.is_string(),
        ParamType::TextOrNumber => value.is_string() || value.is_number(),
        ParamType::Numeric => leading_integer(value).is_some(),
        ParamType::PositiveNumeric => leading_integer(value).is_some_and(|number| number > 0.0),
        ParamType::Guid(length) => value.as_str().is_some_and(|text| {
            let text = text.trim();
            text.len() == length
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }),
        ParamType::Word => value.as_str().is_some_and(|text| {
            !text.is_empty()
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+')
        }),
    }
}

/// What JavaScript's `parseInt` makes of the value, when it is a number.
fn leading_integer(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64().map(f64::trunc),
        Value::String(text) => {
            let text = text.trim_start();
            let (sign, digits) = match text.strip_prefix('-') {
                Some(rest) => (-1.0, rest),
                None => (1.0, text.strip_prefix('+').unwrap_or(text)),
            };
            let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<f64>().ok().map(|number| sign * number)
        }
        _ => None,
    }
}

fn describe_type(expected: ParamType) -> String {
    match expected {
        ParamType::Text | ParamType::OptionalText => "a string".into(),
        ParamType::TextOrNumber => "a string or a number".into(),
        ParamType::Numeric => "a number".into(),
        ParamType::PositiveNumeric => "a number above zero".into(),
        ParamType::Guid(length) => format!("a {length}-character id"),
        ParamType::Word => "letters, digits and underscores only".into(),
    }
}

fn describe_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "a boolean".into(),
        Value::Number(number) => format!("the number {number}"),
        Value::String(text) => format!("\"{}\"", shorten(text)),
        Value::Array(_) => "an array".into(),
        Value::Object(_) => "an object".into(),
    }
}

fn test_ids(adapter: &AdapterSpec, bids: &[BidConfig]) -> Vec<DiagnosticCheck> {
    adapter
        .test_ids
        .iter()
        .filter_map(|test| {
            let using: Vec<&str> = bids
                .iter()
                .filter(|bid| {
                    scalar_text(bid.params.get(test.param)).as_deref() == Some(test.value)
                })
                .map(|bid| bid.ad_unit)
                .collect();
            if using.is_empty() {
                return None;
            }
            let mut check = check(
                DiagnosticArea::Params,
                DiagnosticStatus::Problem,
                codes::PARAMS_TEST_ID,
                format!(
                    "`{}: {}` is the test id from the adapter's documentation, used on {}",
                    test.param,
                    test.value,
                    share_of_ad_units(using.len(), bids.len())
                ),
            );
            check.ad_units = unique(&using);
            Some(check)
        })
        .collect()
}

/// One placement id on every ad unit, although the ad units differ in size.
fn shared_placement_id(adapter: &AdapterSpec, bids: &[BidConfig]) -> Option<DiagnosticCheck> {
    let param = adapter.placement_param?;
    let ad_units: BTreeSet<&str> = bids.iter().map(|bid| bid.ad_unit).collect();
    let size_sets: BTreeSet<&[String]> = bids.iter().map(|bid| bid.sizes).collect();
    let values: BTreeSet<String> = bids
        .iter()
        .map(|bid| scalar_text(bid.params.get(param)).unwrap_or_default())
        .collect();
    let shared = values.first().filter(|value| !value.is_empty())?;
    let is_test_id = adapter
        .test_ids
        .iter()
        .any(|test| test.param == param && test.value == shared.as_str());
    if values.len() != 1
        || ad_units.len() < MIN_AD_UNITS_FOR_SHARED_ID
        || size_sets.len() < 2
        || is_test_id
    {
        return None;
    }
    let mut check = check(
        DiagnosticArea::Params,
        DiagnosticStatus::Notice,
        codes::PARAMS_SHARED_ID,
        format!(
            "All {} ad units use the same `{param}` ({}) although their sizes differ",
            ad_units.len(),
            shorten(shared)
        ),
    );
    check.ad_units = ad_units.into_iter().map(str::to_string).collect();
    Some(check)
}

/// JavaScript truthiness, which is what the adapters test.
fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// A string or number param as text, so `13144370` and `"13144370"` compare equal.
pub(super) fn scalar_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn shorten(text: &str) -> String {
    if text.chars().count() > MAX_VALUE_CHARS {
        format!(
            "{}…",
            text.chars().take(MAX_VALUE_CHARS).collect::<String>()
        )
    } else {
        text.to_string()
    }
}

fn code_list(keys: &[&str]) -> String {
    keys.iter()
        .map(|key| format!("`{key}`"))
        .collect::<Vec<_>>()
        .join(" and ")
}

fn share_of_ad_units(part: usize, whole: usize) -> String {
    if part == whole {
        all_ad_units(whole)
    } else {
        format!("{part} of {whole} ad units")
    }
}

fn all_ad_units(count: usize) -> String {
    match count {
        1 => "its 1 ad unit".into(),
        count => format!("all {count} ad units"),
    }
}

fn unique(ad_units: &[&str]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ad_units
        .iter()
        .filter(|ad_unit| seen.insert(**ad_unit))
        .map(|ad_unit| ad_unit.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::adapters::adapter_for;
    use super::*;

    struct Unit {
        code: &'static str,
        sizes: Vec<String>,
        params: Value,
    }

    fn unit(code: &'static str, size: &str, params: Value) -> Unit {
        Unit {
            code,
            sizes: vec![size.to_string()],
            params,
        }
    }

    fn run(bidder: &str, units: &[Unit]) -> Vec<DiagnosticCheck> {
        run_with_config(bidder, units, &[])
    }

    fn run_with_config(bidder: &str, units: &[Unit], config: &[&str]) -> Vec<DiagnosticCheck> {
        let bids: Vec<BidConfig> = units
            .iter()
            .map(|unit| BidConfig {
                ad_unit: unit.code,
                sizes: &unit.sizes,
                params: &unit.params,
            })
            .collect();
        check_params(adapter_for(bidder).unwrap(), &bids, |key| {
            config.contains(&key)
        })
    }

    fn outcomes(checks: &[DiagnosticCheck]) -> Vec<(&str, &str)> {
        checks
            .iter()
            .map(|check| (check.code.as_str(), check.summary.as_str()))
            .collect()
    }

    #[test]
    fn valid_params_pass_for_every_shape_the_adapters_accept() {
        let cases = [
            ("appnexus", json!({ "placementId": 21804213 })),
            ("appnexus", json!({ "placement_id": "21804213" })),
            ("appnexus", json!({ "member": "958", "invCode": "top" })),
            (
                "rubicon",
                json!({ "accountId": "1001", "siteId": 2002, "zoneId": 3003 }),
            ),
            ("ix", json!({ "siteId": "123456" })),
            ("ix", json!({ "siteId": 123456 })),
            ("pubmatic", json!({ "publisherId": "160262" })),
            ("criteo", json!({ "networkId": 7100 })),
            ("criteo", json!({ "zoneId": 1 })),
            (
                "openx",
                json!({ "delDomain": "pub-d.openx.net", "unit": "5" }),
            ),
            ("openx", json!({ "platform": "abc" })),
            ("triplelift", json!({ "inventoryCode": "site_top" })),
            ("sovrn", json!({ "tagid": "500123" })),
            (
                "seedtag",
                json!({ "publisherId": "1234-5678-01", "adUnitId": "9" }),
            ),
            ("adform", json!({ "mid": 700 })),
            ("adf", json!({ "inv": 1, "mname": "top" })),
            (
                "smartadserver",
                json!({ "siteId": 1, "pageId": 2, "formatId": 3 }),
            ),
            ("teads", json!({ "placementId": 5, "pageId": "6" })),
            (
                "improvedigital",
                json!({ "placementId": 5, "publisherId": 6 }),
            ),
            ("sharethrough", json!({ "pkey": "abc" })),
            ("amx", json!({})),
            ("amx", json!({ "tagId": "abc", "endpoint": null })),
            ("medianet", json!({ "cid": "8CU123ABC" })),
            (
                "ttd",
                json!({ "supplySourceId": "acme", "publisherId": "1" }),
            ),
            ("yieldmo", json!({})),
            ("gumgum", json!({ "zone": "abc" })),
            ("gumgum", json!({ "inSlot": 9 })),
            ("onetag", json!({ "pubId": "386276e072" })),
            ("richaudience", json!({ "pid": "abc" })),
            ("unruly", json!({ "siteId": 1 })),
            ("kargo", json!({ "placementId": "_abc" })),
            ("33across", json!({ "siteId": "abcdefghij_klmnopqr-12" })),
        ];
        for (bidder, params) in cases {
            let checks = run(bidder, &[unit("top", "728x90", params.clone())]);
            assert_eq!(
                checks.iter().map(|check| check.status).collect::<Vec<_>>(),
                [DiagnosticStatus::Pass],
                "{bidder} {params}: {:?}",
                outcomes(&checks)
            );
        }
    }

    #[test]
    fn missing_required_params_are_named_with_the_ad_units_they_are_missing_on() {
        let checks = run(
            "appnexus",
            &[
                unit("top", "728x90", json!({ "placementId": 21804213 })),
                unit("rail", "300x250", json!({})),
                unit("article", "336x280", json!({ "placementId": 0 })),
                unit("footer", "320x50", json!({ "placementId": "" })),
            ],
        );
        assert_eq!(
            outcomes(&checks),
            [(
                "params-missing",
                "`placementId` missing on 3 of 4 ad units (the adapter needs `placementId` or `member` with `invCode`)"
            )]
        );
        assert_eq!(checks[0].status, DiagnosticStatus::Problem);
        assert_eq!(checks[0].ad_units, ["rail", "article", "footer"]);
    }

    #[test]
    fn the_closest_alternative_decides_what_is_reported_missing() {
        let checks = run(
            "appnexus",
            &[unit("top", "728x90", json!({ "member": "958" }))],
        );
        // `member` alone is one param short of the member route, as is an
        // empty object of the placement route; the first alternative wins a tie.
        assert_eq!(checks[0].code, "params-missing");
        assert!(
            checks[0]
                .summary
                .starts_with("`placementId` missing on its 1 ad unit"),
            "{}",
            checks[0].summary
        );

        let rubicon = run(
            "rubicon",
            &[
                unit("top", "728x90", json!({ "accountId": 1001 })),
                unit(
                    "rail",
                    "300x250",
                    json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
                ),
            ],
        );
        assert_eq!(
            outcomes(&rubicon),
            [(
                "params-missing",
                "`siteId` and `zoneId` missing on 1 of 2 ad units"
            )]
        );
    }

    #[test]
    fn wrong_types_follow_the_adapter_source() {
        let cases = [
            (
                "pubmatic",
                json!({ "publisherId": 160262 }),
                "`publisherId` must be a string, found the number 160262 on its 1 ad unit",
            ),
            (
                "rubicon",
                json!({ "accountId": "acct", "siteId": 2, "zoneId": 3 }),
                "`accountId` must be a number, found \"acct\" on its 1 ad unit",
            ),
            (
                "sovrn",
                json!({ "tagid": "tag-1" }),
                "`tagid` must be a number, found \"tag-1\" on its 1 ad unit",
            ),
            (
                "teads",
                json!({ "placementId": -4, "pageId": 6 }),
                "`placementId` must be a number above zero, found the number -4 on its 1 ad unit",
            ),
            (
                "onetag",
                json!({ "pubId": 386276 }),
                "`pubId` must be a string, found the number 386276 on its 1 ad unit",
            ),
            (
                "33across",
                json!({ "siteId": "too-short" }),
                "`siteId` must be a 22-character id, found \"too-short\" on its 1 ad unit",
            ),
            (
                "ttd",
                json!({ "supplySourceId": "acme inc", "publisherId": "1" }),
                "`supplySourceId` must be letters, digits and underscores only, found \"acme inc\" on its 1 ad unit",
            ),
            (
                "amx",
                json!({ "tagId": 12 }),
                "`tagId` must be a string, found the number 12 on its 1 ad unit",
            ),
            (
                "ix",
                json!({ "siteId": { "id": 1 } }),
                "`siteId` must be a string or a number, found an object on its 1 ad unit",
            ),
        ];
        for (bidder, params, expected) in cases {
            let checks = run(bidder, &[unit("top", "728x90", params)]);
            assert_eq!(
                outcomes(&checks),
                [("params-wrong-type", expected)],
                "{bidder}"
            );
            assert_eq!(checks[0].ad_units, ["top"]);
        }
    }

    #[test]
    fn index_exchange_needs_no_site_id_when_an_exchange_id_is_configured() {
        let units = [unit("top", "728x90", json!({}))];
        assert_eq!(run("ix", &units)[0].code, "params-missing");
        assert_eq!(
            run_with_config("ix", &units, &["exchangeId"])[0].code,
            "params-ok"
        );
    }

    #[test]
    fn documented_test_ids_are_flagged_whether_written_as_number_or_string() {
        let cases = [
            ("appnexus", json!({ "placementId": 13144370 })),
            ("appnexus", json!({ "placementId": "13144370" })),
            (
                "rubicon",
                json!({ "accountId": 14062, "siteId": 70608, "zoneId": 498816 }),
            ),
            ("pubmatic", json!({ "publisherId": "156209" })),
            ("seedtag", json!({ "publisherId": "0000-0000-01" })),
            ("medianet", json!({ "cid": "8CUX0H51C" })),
            ("amx", json!({ "tagId": "cHJlYmlkLm9yZw" })),
        ];
        for (bidder, params) in cases {
            let checks = run(bidder, &[unit("top", "728x90", params.clone())]);
            assert_eq!(
                checks
                    .iter()
                    .map(|check| check.code.as_str())
                    .collect::<Vec<_>>(),
                ["params-test-id"],
                "{bidder} {params}"
            );
        }
        let near_miss = run(
            "appnexus",
            &[unit("top", "728x90", json!({ "placementId": 13144371 }))],
        );
        assert_eq!(near_miss[0].code, "params-ok");
    }

    #[test]
    fn one_placement_id_on_differently_sized_ad_units_is_a_notice() {
        let shared = |sizes: [&str; 3]| {
            run(
                "appnexus",
                &[
                    unit("top", sizes[0], json!({ "placementId": 21804213 })),
                    unit("rail", sizes[1], json!({ "placementId": "21804213" })),
                    unit("footer", sizes[2], json!({ "placementId": 21804213 })),
                ],
            )
        };
        let checks = shared(["728x90", "300x250", "320x50"]);
        assert_eq!(
            outcomes(&checks),
            [(
                "params-shared-id",
                "All 3 ad units use the same `placementId` (21804213) although their sizes differ"
            )]
        );
        assert_eq!(checks[0].status, DiagnosticStatus::Notice);

        // The same size everywhere is one placement shown three times.
        assert_eq!(
            shared(["300x250", "300x250", "300x250"])[0].code,
            "params-ok"
        );
        // Two ad units are too few to call it a pattern.
        let two = run(
            "appnexus",
            &[
                unit("top", "728x90", json!({ "placementId": 1 })),
                unit("rail", "300x250", json!({ "placementId": 1 })),
            ],
        );
        assert_eq!(two[0].code, "params-ok");
        // Rubicon zones are shared between placements by design.
        let zones = run(
            "rubicon",
            &[
                unit(
                    "top",
                    "728x90",
                    json!({ "accountId": 1, "siteId": 2, "zoneId": 3 }),
                ),
                unit(
                    "rail",
                    "300x250",
                    json!({ "accountId": 1, "siteId": 2, "zoneId": 3 }),
                ),
                unit(
                    "footer",
                    "320x50",
                    json!({ "accountId": 1, "siteId": 2, "zoneId": 3 }),
                ),
            ],
        );
        assert_eq!(zones[0].code, "params-ok");
    }

    #[test]
    fn parse_int_semantics_match_javascript() {
        let cases = [
            (json!(12), Some(12.0)),
            (json!(12.9), Some(12.0)),
            (json!("12abc"), Some(12.0)),
            (json!(" 7"), Some(7.0)),
            (json!("-3"), Some(-3.0)),
            (json!("abc"), None),
            (json!(""), None),
            (json!(true), None),
            (json!(null), None),
        ];
        for (value, expected) in cases {
            assert_eq!(leading_integer(&value), expected, "{value}");
        }
    }
}
