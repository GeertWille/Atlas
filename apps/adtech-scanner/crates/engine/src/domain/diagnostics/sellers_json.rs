//! Looks seller ids up in a sellers.json file (IAB Tech Lab sellers.json 1.0)
//! and compares what the exchange declares with the site's ads.txt.

use std::fmt;

use adscan_model::{DiagnosticArea, DiagnosticCheck, DiagnosticStatus};
use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::ads_txt::{Record, Relationship};
use super::{check, codes, excerpt};

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
    let mut deserializer = serde_json::Deserializer::from_slice(json);
    deserializer
        .deserialize_map(FileVisitor { wanted })
        .map_err(|error| format!("the file is not valid sellers.json ({error})"))
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
            let Some(seller_id) = entry.seller_id.map(IdText::into_text) else {
                continue;
            };
            if self
                .wanted
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(&seller_id))
            {
                found.push(Seller {
                    seller_id,
                    seller_type: entry.seller_type.as_deref().and_then(seller_type),
                    is_confidential: entry.is_confidential.is_some_and(Flag::is_set),
                    domain: entry
                        .domain
                        .map(|domain| domain.trim().to_ascii_lowercase())
                        .filter(|domain| !domain.is_empty()),
                });
            }
        }
        Ok(found)
    }
}

/// Real files write the id as a string or a number and the confidentiality
/// flag as 0/1 or a boolean.
#[derive(Deserialize)]
struct RawSeller {
    seller_id: Option<IdText>,
    seller_type: Option<String>,
    is_confidential: Option<Flag>,
    domain: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IdText {
    Text(String),
    Number(serde_json::Number),
}

impl IdText {
    fn into_text(self) -> String {
        match self {
            Self::Text(text) => text.trim().to_string(),
            Self::Number(number) => number.to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Number(f64),
}

impl Flag {
    fn is_set(self) -> bool {
        match self {
            Self::Bool(flag) => flag,
            Self::Number(number) => number != 0.0,
        }
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
    let Some(seller) = seller else {
        let mut check = check(
            DiagnosticArea::SellersJson,
            DiagnosticStatus::Problem,
            codes::SELLERS_ID_MISSING,
            format!(
                "Seller id {} from ads.txt is not in the sellers.json of {}",
                record.account_id, record.domain
            ),
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
                "{} lists seller id {} as confidential, so buyers cannot see who sells this inventory",
                record.domain, record.account_id
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
                "ads.txt lists seller id {} at {} as {}, but its sellers.json lists it as {}",
                record.account_id,
                record.domain,
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
                    "{} lists seller id {} as the publisher {domain}, not {}; add OWNERDOMAIN to ads.txt if {domain} owns this site",
                    record.domain, record.account_id, site.domain
                ),
            );
        }
    }
    if checks.is_empty() {
        let mut check = check(
            DiagnosticArea::SellersJson,
            DiagnosticStatus::Pass,
            codes::SELLERS_OK,
            format!(
                "Seller id {} is listed in the sellers.json of {}",
                record.account_id, record.domain
            ),
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
        let cases: [(&str, &str, &[&str], &[(DiagnosticStatus, &str)]); 8] = [
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
