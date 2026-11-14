//! Looks seller ids up in a sellers.json file (IAB Tech Lab sellers.json 1.0)
//! and compares what the exchange declares with the site's ads.txt.

use std::fmt;

use adscan_model::{DiagnosticArea, DiagnosticCheck, DiagnosticStatus};
use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

use super::ads_txt::{Record, Relationship};
use super::{brief, check, codes, excerpt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellerType {
    Publisher,
    Intermediary,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seller {
    pub seller_id: String,
    pub seller_type: Option<SellerType>,
    pub is_confidential: bool,
    /// Lower-cased business domain of the seller.
    pub domain: Option<String>,
}

/// What became of the sellers.json of one ad system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SellersOutcome {
    /// The file was read; these are the entries of the ids asked for.
    Read(Vec<Seller>),
    /// No conclusion is possible; the text says why.
    Unavailable(String),
}

/// The entries of `wanted` seller ids. The file can list tens of thousands of
/// sellers, so everything else is skipped while parsing instead of collected.
pub fn find_sellers(json: &[u8], wanted: &[String]) -> Result<Vec<Seller>, String> {
    // Some servers put a byte order mark in front, which JSON does not allow.
    let json = json.strip_prefix(b"\xef\xbb\xbf").unwrap_or(json);
    let mut deserializer = serde_json::Deserializer::from_slice(json);
    deserializer
        .deserialize_map(FileVisitor { wanted })
        .map_err(|error| format!("the file is not valid sellers.json ({error})"))
}

/// Whether an answer is a web page (a firewall's block page, a challenge or
/// an error page) rather than a file: announced as HTML, or markup however it
/// is announced. No sellers.json starts with a tag.
pub fn is_web_page(content_type: Option<&str>, body: &[u8]) -> bool {
    let html = content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("text/html"));
    let body = body.strip_prefix(b"\xef\xbb\xbf").unwrap_or(body);
    html || body.trim_ascii_start().starts_with(b"<")
}

/// Whether `json` is a sellers.json at all: what `find_sellers` can read,
/// whichever ids are asked for. A web page, a soft 404, JSON without a
/// `sellers` array and a file that was cut short are not.
pub fn check_readable(json: &[u8]) -> Result<(), String> {
    find_sellers(json, &[]).map(drop)
}

struct FileVisitor<'a> {
    wanted: &'a [String],
}

impl<'de> Visitor<'de> for FileVisitor<'_> {
    type Value = Vec<Seller>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sellers.json object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut found = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "sellers" {
                found = Some(map.next_value_seed(SellersSeed {
                    wanted: self.wanted,
                })?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        found.ok_or_else(|| serde::de::Error::missing_field("sellers"))
    }
}

struct SellersSeed<'a> {
    wanted: &'a [String],
}

impl<'de> DeserializeSeed<'de> for SellersSeed<'_> {
    type Value = Vec<Seller>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for SellersSeed<'_> {
    type Value = Vec<Seller>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("the sellers array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut found = Vec::new();
        while let Some(entry) = sequence.next_element::<RawSeller>()? {
            let Some(seller_id) = entry.seller_id.as_ref().and_then(id_text) else {
                continue;
            };
            if self
                .wanted
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(&seller_id))
            {
                found.push(Seller {
                    seller_id,
                    seller_type: entry
                        .seller_type
                        .as_ref()
                        .and_then(Value::as_str)
                        .and_then(seller_type),
                    is_confidential: entry.is_confidential.as_ref().is_some_and(is_set),
                    domain: entry
                        .domain
                        .as_ref()
                        .and_then(Value::as_str)
                        .map(|domain| domain.trim().to_ascii_lowercase())
                        .filter(|domain| !domain.is_empty()),
                });
            }
        }
        Ok(found)
    }
}

/// The fields of one seller entry, in whatever JSON type the file gives
/// them: one entry with a field of the wrong type must not make the other
/// tens of thousands unreadable.
#[derive(Deserialize)]
struct RawSeller {
    seller_id: Option<Value>,
    seller_type: Option<Value>,
    is_confidential: Option<Value>,
    domain: Option<Value>,
}

/// Real files write the id as a string or a number.
fn id_text(id: &Value) -> Option<String> {
    match id {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Real files write the confidentiality flag as 0/1 or a boolean.
fn is_set(flag: &Value) -> bool {
    match flag {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        _ => false,
    }
}

fn seller_type(text: &str) -> Option<SellerType> {
    match text.trim().to_ascii_uppercase().as_str() {
        "PUBLISHER" => Some(SellerType::Publisher),
        "INTERMEDIARY" => Some(SellerType::Intermediary),
        "BOTH" => Some(SellerType::Both),
        _ => None,
    }
}

/// The domains that may own the scanned site's inventory.
pub struct SiteIdentity<'a> {
    /// Registrable domain of the scanned page.
    pub domain: &'a str,
    /// OWNERDOMAIN and MANAGERDOMAIN of its ads.txt.
    pub declared_domains: &'a [String],
}

/// Compares one ads.txt record with the exchange's entry for its account id.
pub fn compare(
    record: &Record,
    seller: Option<&Seller>,
    site: &SiteIdentity,
) -> Vec<DiagnosticCheck> {
    let line = Some(excerpt(&record.text));
    let account = brief(&record.account_id);
    let exchange = brief(&record.domain);
    let Some(seller) = seller else {
        let mut check = check(
            DiagnosticArea::SellersJson,
            DiagnosticStatus::Problem,
            codes::SELLERS_ID_MISSING,
            format!("Seller id {account} from ads.txt is not in the sellers.json of {exchange}"),
        );
        check.excerpt = line;
        return vec![check];
    };
    let mut checks = Vec::new();
    let mut notice = |code: &str, summary: String| {
        let mut check = check(
            DiagnosticArea::SellersJson,
            DiagnosticStatus::Notice,
            code,
            summary,
        );
        check.excerpt.clone_from(&line);
        checks.push(check);
    };
    if seller.is_confidential {
        notice(
            codes::SELLERS_CONFIDENTIAL,
            format!(
                "{exchange} lists seller id {account} as confidential, so buyers cannot see who sells this inventory"
            ),
        );
    }
    let type_matches = match (record.relationship, seller.seller_type) {
        (_, None | Some(SellerType::Both)) => true,
        (Relationship::Direct, Some(seller_type)) => seller_type == SellerType::Publisher,
        (Relationship::Reseller, Some(seller_type)) => seller_type == SellerType::Intermediary,
    };
    if !type_matches {
        notice(
            codes::SELLERS_TYPE_MISMATCH,
            format!(
                "ads.txt lists seller id {account} at {exchange} as {}, but its sellers.json lists it as {}",
                record.relationship.as_str(),
                match seller.seller_type {
                    Some(SellerType::Publisher) => "PUBLISHER",
                    _ => "INTERMEDIARY",
                },
            ),
        );
    }
    // Only a direct seller that is a publisher is expected to be the site
    // itself; an intermediary's entry names the intermediary.
    let should_be_the_site = record.relationship == Relationship::Direct
        && seller.seller_type == Some(SellerType::Publisher)
        && !seller.is_confidential;
    if let Some(domain) = seller.domain.as_deref().filter(|_| should_be_the_site) {
        let seller_root = psl::domain_str(domain).unwrap_or(domain);
        let is_the_site = seller_root == site.domain
            || site
                .declared_domains
                .iter()
                .any(|declared| declared == domain || declared == seller_root);
        if !is_the_site {
            notice(
                codes::SELLERS_DOMAIN_MISMATCH,
                format!(
                    "{exchange} lists seller id {account} as the publisher {seller_domain}, not {}; add OWNERDOMAIN to ads.txt if {seller_domain} owns this site",
                    site.domain,
                    seller_domain = brief(domain)
                ),
            );
        }
    }
    if checks.is_empty() {
        let mut check = check(
            DiagnosticArea::SellersJson,
            DiagnosticStatus::Pass,
            codes::SELLERS_OK,
            format!("Seller id {account} is listed in the sellers.json of {exchange}"),
        );
        check.excerpt = line;
        checks.push(check);
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::super::ads_txt;
    use super::*;

    const FILE: &str = r#"{
        "contact_email": "sellers@exchange.example",
        "version": "1.0",
        "identifiers": [{ "name": "TAG-ID", "value": "abc" }],
        "sellers": [
            { "seller_id": "958", "name": "Example Media", "domain": "Example-Media.com", "seller_type": "PUBLISHER" },
            { "seller_id": 1001, "seller_type": "intermediary", "domain": "reseller.example", "is_confidential": 0 },
            { "seller_id": "c-7", "is_confidential": 1, "seller_type": "PUBLISHER" },
            { "seller_id": "b-8", "is_confidential": true, "seller_type": "Both" },
            { "name": "no id" },
            { "seller_id": "x-9", "seller_type": "NA", "domain": " " }
        ]
    }"#;

    fn wanted(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn only_the_wanted_sellers_are_returned_in_every_shape_files_use() {
        let sellers = find_sellers(
            FILE.as_bytes(),
            &wanted(&["958", "1001", "C-7", "b-8", "x-9", "nope"]),
        )
        .unwrap();
        assert_eq!(
            sellers,
            [
                Seller {
                    seller_id: "958".into(),
                    seller_type: Some(SellerType::Publisher),
                    is_confidential: false,
                    domain: Some("example-media.com".into()),
                },
                Seller {
                    seller_id: "1001".into(),
                    seller_type: Some(SellerType::Intermediary),
                    is_confidential: false,
                    domain: Some("reseller.example".into()),
                },
                Seller {
                    seller_id: "c-7".into(),
                    seller_type: Some(SellerType::Publisher),
                    is_confidential: true,
                    domain: None,
                },
                Seller {
                    seller_id: "b-8".into(),
                    seller_type: Some(SellerType::Both),
                    is_confidential: true,
                    domain: None,
                },
                Seller {
                    seller_id: "x-9".into(),
                    seller_type: None,
                    is_confidential: false,
                    domain: None,
                },
            ]
        );
        assert_eq!(
            find_sellers(FILE.as_bytes(), &wanted(&["nope"])),
            Ok(vec![])
        );
    }

    #[test]
    fn anything_that_is_not_sellers_json_is_an_error() {
        for body in [
            "",
            "<html>",
            "[]",
            r#"{"version":"1.0"}"#,
            r#"{"sellers":{}}"#,
            r#"{"sellers":[{"seller_id":"1"}"#,
        ] {
            let error = find_sellers(body.as_bytes(), &wanted(&["1"])).unwrap_err();
            assert!(
                error.starts_with("the file is not valid sellers.json"),
                "{body}: {error}"
            );
        }
    }

    #[test]
    fn a_file_is_readable_exactly_when_a_lookup_can_read_it() {
        for sellers_json in [
            r#"{"sellers":[]}"#,
            r#"{"version":"1.0","sellers":[{"seller_id":"958"},{"seller_id":7,"domain":null}]}"#,
            "\u{feff}{\"sellers\":[]}",
        ] {
            assert_eq!(
                check_readable(sellers_json.as_bytes()),
                Ok(()),
                "{sellers_json}"
            );
        }
        for other in [
            "<!DOCTYPE html><html><body>Just a moment...</body></html>",
            "Not Found",
            "",
            r#"{"error":"not found"}"#,
            r#"{"sellers":{"958":{}}}"#,
            r#"{"sellers":[{"seller_id":"958"},{"seller_"#,
        ] {
            let reason = check_readable(other.as_bytes()).unwrap_err();
            assert!(
                reason.starts_with("the file is not valid sellers.json ("),
                "{other}: {reason}"
            );
            // The same verdict, in the same words, as the lookup itself.
            assert_eq!(
                find_sellers(other.as_bytes(), &wanted(&["958"])),
                Err(reason)
            );
        }
    }

    #[test]
    fn one_odd_entry_does_not_make_the_file_unreadable() {
        let file = r#"{
            "sellers": [
                { "seller_id": null, "domain": "a.example" },
                { "seller_id": true },
                { "seller_id": ["7"] },
                { "seller_id": { "id": "7" } },
                { "seller_id": "" },
                { "seller_id": " 7 ", "seller_type": 3, "domain": 12, "is_confidential": "yes" },
                { "seller_id": 7, "seller_type": "PUBLISHER", "domain": null, "is_confidential": null },
                { "seller_id": 7.5, "name": { "deep": [[[{}]]] }, "ext": [1, 2, 3] },
                { "seller_id": 18446744073709551615 },
                {}
            ],
            "sellers_note": "the key above is the only one read",
            "ext": { "sellers": "not this one" }
        }"#;
        let sellers = find_sellers(
            file.as_bytes(),
            &wanted(&["7", "7.5", "18446744073709551615", "", "true"]),
        )
        .unwrap();
        let found: Vec<(&str, Option<SellerType>, bool, Option<&str>)> = sellers
            .iter()
            .map(|seller| {
                (
                    seller.seller_id.as_str(),
                    seller.seller_type,
                    seller.is_confidential,
                    seller.domain.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("7", None, false, None),
                ("7", Some(SellerType::Publisher), false, None),
                ("7.5", None, false, None),
                ("18446744073709551615", None, false, None),
            ]
        );
    }

    #[test]
    fn truncated_malformed_and_hostile_files_are_errors_not_panics() {
        let deeply_nested = format!("{{\"sellers\":{}", "[".repeat(100_000));
        let many_sellers = format!(
            "{{\"sellers\":[{}",
            r#"{"seller_id":"1","name":"x"},"#.repeat(50_000)
        );
        for body in [
            r#"{"sellers":["#,
            r#"{"sellers":[{"seller_id":"1","#,
            r#"{"sellers":[{"seller_id":"1"},]}"#,
            r#"{"sellers":[null]}"#,
            r#"{"sellers":["1"]}"#,
            r#"{"sellers":null}"#,
            r#"{"sellers":"none"}"#,
            r#"{"SELLERS":[]}"#,
            "null",
            "1",
            "\"sellers\"",
            deeply_nested.as_str(),
            many_sellers.as_str(),
        ] {
            let outcome = find_sellers(body.as_bytes(), &wanted(&["1"]));
            assert!(
                outcome
                    .as_ref()
                    .is_err_and(|error| error.starts_with("the file is not valid sellers.json")),
                "{}: {outcome:?}",
                &body[..body.len().min(40)]
            );
        }
        assert!(find_sellers(&[0xff, 0xfe, 0x00], &wanted(&["1"])).is_err());
        assert_eq!(
            find_sellers("\u{feff}{\"sellers\":[]}".as_bytes(), &wanted(&["1"])),
            Ok(vec![]),
            "a byte order mark is not part of the file"
        );
    }

    #[test]
    fn a_large_file_is_searched_without_keeping_what_is_not_wanted() {
        let mut file = String::from(r#"{"version":"1.0","sellers":["#);
        for id in 0..200_000 {
            file.push_str(&format!(
                r#"{{"seller_id":"{id}","name":"Seller {id}","domain":"s{id}.example","seller_type":"PUBLISHER"}},"#
            ));
        }
        file.push_str(r#"{"seller_id":"199999","seller_type":"BOTH"}]}"#);
        let sellers = find_sellers(file.as_bytes(), &wanted(&["199999", "17"])).unwrap();
        let ids: Vec<&str> = sellers
            .iter()
            .map(|seller| seller.seller_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["17", "199999", "199999"],
            "duplicates are kept in file order"
        );
        assert_eq!(sellers[1].seller_type, Some(SellerType::Publisher));
    }

    fn record(line: &str) -> Record {
        ads_txt::parse(line).records.remove(0)
    }

    fn judge(line: &str, seller_id: &str, declared: &[&str]) -> Vec<(DiagnosticStatus, String)> {
        let sellers = find_sellers(FILE.as_bytes(), &wanted(&[seller_id])).unwrap();
        let declared = wanted(declared);
        compare(
            &record(line),
            sellers.first(),
            &SiteIdentity {
                domain: "news.example",
                declared_domains: &declared,
            },
        )
        .into_iter()
        .map(|check| (check.status, check.code))
        .collect()
    }

    #[test]
    fn records_are_compared_with_the_exchanges_entry() {
        use DiagnosticStatus::{Notice, Pass, Problem};
        /// ads.txt line, seller id looked up, declared owner domains, expected checks.
        type Case = (
            &'static str,
            &'static str,
            &'static [&'static str],
            &'static [(DiagnosticStatus, &'static str)],
        );
        let cases: [Case; 8] = [
            (
                "exchange.example, 404, DIRECT",
                "404",
                &[],
                &[(Problem, "sellers-json-id-missing")],
            ),
            (
                "exchange.example, 958, DIRECT",
                "958",
                &["example-media.com"],
                &[(Pass, "sellers-json-ok")],
            ),
            (
                "exchange.example, 958, DIRECT",
                "958",
                &[],
                &[(Notice, "sellers-json-domain-mismatch")],
            ),
            (
                "exchange.example, 958, RESELLER",
                "958",
                &["example-media.com"],
                &[(Notice, "sellers-json-type-mismatch")],
            ),
            (
                "exchange.example, 1001, RESELLER",
                "1001",
                &[],
                &[(Pass, "sellers-json-ok")],
            ),
            (
                "exchange.example, 1001, DIRECT",
                "1001",
                &[],
                &[(Notice, "sellers-json-type-mismatch")],
            ),
            (
                "exchange.example, c-7, DIRECT",
                "c-7",
                &[],
                &[(Notice, "sellers-json-confidential")],
            ),
            (
                "exchange.example, b-8, RESELLER",
                "b-8",
                &[],
                &[(Notice, "sellers-json-confidential")],
            ),
        ];
        for (line, seller_id, declared, expected) in cases {
            let expected: Vec<(DiagnosticStatus, String)> = expected
                .iter()
                .map(|(status, code)| (*status, code.to_string()))
                .collect();
            assert_eq!(judge(line, seller_id, declared), expected, "{line}");
        }
    }

    #[test]
    fn a_seller_on_a_subdomain_of_the_site_is_the_site() {
        let seller = Seller {
            seller_id: "1".into(),
            seller_type: Some(SellerType::Publisher),
            is_confidential: false,
            domain: Some("www.news.example".into()),
        };
        let checks = compare(
            &record("exchange.example, 1, DIRECT"),
            Some(&seller),
            &SiteIdentity {
                domain: "news.example",
                declared_domains: &[],
            },
        );
        assert_eq!(checks[0].code, "sellers-json-ok");
        assert_eq!(
            checks[0].excerpt.as_deref(),
            Some("exchange.example, 1, DIRECT")
        );
    }
}
