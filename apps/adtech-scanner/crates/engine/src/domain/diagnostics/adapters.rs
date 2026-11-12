//! What the scanner knows about the common Prebid bid adapters.
//!
//! Required params, param types, aliases and GVL ids were read from the
//! adapter sources of Prebid.js 11.49.0 (`modules/<bidder>BidAdapter.js`:
//! `isBidRequestValid`, `aliases`, `gvlid`). Documented test ids come from the
//! "Test Parameters" of the adapter's `.md` file in the same release. GVL ids
//! were cross-checked against the IAB Global Vendor List v184. The ads.txt
//! domains were checked against the ads.txt files of large publishers; an
//! adapter whose ad system domain or account id param is not certain has none.

/// How a param value must look for the adapter to accept the bid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// A JavaScript string (a number is rejected).
    Text,
    /// A string, or absent.
    OptionalText,
    /// A number or a string.
    TextOrNumber,
    /// A number, or a string that starts with digits (`parseInt` succeeds).
    Numeric,
    /// As `Numeric`, and greater than zero.
    PositiveNumeric,
    /// A string of exactly this many characters from `[A-Za-z0-9_-]`.
    Guid(usize),
    /// Letters, digits, `_` and `+` only, as a string or a number.
    Word,
}

/// A value the adapter documents as a test id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestId {
    pub param: &'static str,
    pub value: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterSpec {
    /// Bidder code of the adapter.
    pub code: &'static str,
    pub aliases: &'static [&'static str],
    /// The ways to satisfy the adapter: one alternative must be complete. An
    /// alternative is a list of params that must all be set; a param is a
    /// list of interchangeable keys. Empty when nothing is required.
    pub required: &'static [&'static [&'static [&'static str]]],
    pub types: &'static [(&'static str, ParamType)],
    /// A Prebid config key that lifts the param requirement.
    pub waived_by_config: Option<&'static str>,
    /// The param that identifies one placement, where sharing it between
    /// ad units is unusual.
    pub placement_param: Option<&'static str>,
    pub test_ids: &'static [TestId],
    /// Domains the ad system uses in ads.txt. Empty when not known for sure.
    pub ad_system_domains: &'static [&'static str],
    /// The param that carries the ads.txt account id, where one does.
    pub account_param: Option<&'static str>,
    /// IAB Global Vendor List id.
    pub gvl_id: u32,
}

const fn spec(code: &'static str, gvl_id: u32) -> AdapterSpec {
    AdapterSpec {
        code,
        aliases: &[],
        required: &[],
        types: &[],
        waived_by_config: None,
        placement_param: None,
        test_ids: &[],
        ad_system_domains: &[],
        account_param: None,
        gvl_id,
    }
}

const fn test_id(param: &'static str, value: &'static str) -> TestId {
    TestId { param, value }
}

pub const ADAPTERS: &[AdapterSpec] = &[
    AdapterSpec {
        // Aliases that carry their own GVL id (adasta, beintoo, adzymic) are
        // other companies and are left out.
        aliases: &[
            "appnexusAst",
            "pagescience",
            "gourmetads",
            "newdream",
            "matomy",
            "featureforward",
            "stailamedia",
            "uol",
        ],
        required: &[
            &[&["placementId", "placement_id"]],
            &[&["member"], &["invCode", "inv_code"]],
        ],
        placement_param: Some("placementId"),
        test_ids: &[test_id("placementId", "13144370")],
        ad_system_domains: &["appnexus.com", "xandr.com"],
        account_param: Some("member"),
        ..spec("appnexus", 32)
    },
    AdapterSpec {
        required: &[&[&["accountId"], &["siteId"], &["zoneId"]]],
        types: &[
            ("accountId", ParamType::Numeric),
            ("siteId", ParamType::Numeric),
            ("zoneId", ParamType::Numeric),
        ],
        test_ids: &[test_id("accountId", "14062")],
        ad_system_domains: &["rubiconproject.com"],
        account_param: Some("accountId"),
        ..spec("rubicon", 52)
    },
    AdapterSpec {
        required: &[&[&["siteId"]]],
        types: &[("siteId", ParamType::TextOrNumber)],
        waived_by_config: Some("exchangeId"),
        ad_system_domains: &["indexexchange.com"],
        ..spec("ix", 10)
    },
    AdapterSpec {
        required: &[&[&["publisherId"]]],
        types: &[("publisherId", ParamType::Text)],
        test_ids: &[test_id("publisherId", "156209")],
        ad_system_domains: &["pubmatic.com"],
        account_param: Some("publisherId"),
        ..spec("pubmatic", 76)
    },
    AdapterSpec {
        required: &[&[&["zoneId"]], &[&["networkId"]]],
        ad_system_domains: &["criteo.com"],
        ..spec("criteo", 91)
    },
    AdapterSpec {
        // `unit` is only required when the ad unit has no banner sizes.
        required: &[&[&["delDomain"]], &[&["platform"]]],
        ad_system_domains: &["openx.com"],
        ..spec("openx", 69)
    },
    AdapterSpec {
        required: &[&[&["inventoryCode"]]],
        ad_system_domains: &["triplelift.com"],
        ..spec("triplelift", 28)
    },
    AdapterSpec {
        required: &[&[&["tagid"]]],
        types: &[("tagid", ParamType::Numeric)],
        placement_param: Some("tagid"),
        test_ids: &[test_id("tagid", "403370")],
        ad_system_domains: &["sovrn.com", "lijit.com"],
        ..spec("sovrn", 13)
    },
    AdapterSpec {
        aliases: &["st"],
        required: &[&[&["publisherId"]]],
        test_ids: &[test_id("publisherId", "0000-0000-01")],
        ad_system_domains: &["seedtag.com"],
        ..spec("seedtag", 157)
    },
    AdapterSpec {
        aliases: &["adform", "adformOpenRTB"],
        required: &[&[&["mid"]], &[&["inv"], &["mname"]]],
        placement_param: Some("mid"),
        ad_system_domains: &["adform.com"],
        ..spec("adf", 50)
    },
    AdapterSpec {
        aliases: &["smart"],
        required: &[&[&["siteId"], &["pageId"], &["formatId"]]],
        ad_system_domains: &["smartadserver.com"],
        ..spec("smartadserver", 45)
    },
    AdapterSpec {
        required: &[&[&["placementId"], &["pageId"]]],
        types: &[
            ("placementId", ParamType::PositiveNumeric),
            ("pageId", ParamType::PositiveNumeric),
        ],
        placement_param: Some("placementId"),
        ad_system_domains: &["teads.tv"],
        ..spec("teads", 132)
    },
    AdapterSpec {
        aliases: &["id"],
        required: &[&[&["placementId"], &["publisherId"]]],
        placement_param: Some("placementId"),
        test_ids: &[test_id("placementId", "22135702")],
        ad_system_domains: &["improvedigital.com"],
        ..spec("improvedigital", 253)
    },
    AdapterSpec {
        required: &[&[&["pkey"]]],
        placement_param: Some("pkey"),
        test_ids: &[test_id("pkey", "LuB3vxGGFrBZJa6tifXW4xgK")],
        ad_system_domains: &["sharethrough.com"],
        ..spec("sharethrough", 80)
    },
    AdapterSpec {
        types: &[
            ("tagId", ParamType::OptionalText),
            ("endpoint", ParamType::OptionalText),
        ],
        test_ids: &[test_id("tagId", "cHJlYmlkLm9yZw")],
        ad_system_domains: &["amxrtb.com"],
        ..spec("amx", 737)
    },
    AdapterSpec {
        required: &[&[&["cid"]]],
        types: &[("cid", ParamType::Text)],
        test_ids: &[test_id("cid", "8CUX0H51C")],
        ad_system_domains: &["media.net"],
        account_param: Some("cid"),
        ..spec("medianet", 142)
    },
    AdapterSpec {
        // `placementId` may be replaced by a GPID, which the scan does not
        // see, so it is not required here. The ads.txt domain of The Trade
        // Desk's supply path is not certain and is left out.
        aliases: &["thetradedesk"],
        required: &[&[&["supplySourceId"], &["publisherId"]]],
        types: &[("supplySourceId", ParamType::Word)],
        ..spec("ttd", 21)
    },
    AdapterSpec {
        ad_system_domains: &["yieldmo.com"],
        ..spec("yieldmo", 173)
    },
    AdapterSpec {
        aliases: &["gg"],
        required: &[
            &[&["zone"]],
            &[&["pubId"]],
            &[&["inScreen"]],
            &[&["inScreenPubID"]],
            &[&["inSlot"]],
            &[&["ICV"]],
            &[&["video"]],
            &[&["inVideo"]],
            &[&["videoPubID"]],
        ],
        ad_system_domains: &["gumgum.com"],
        ..spec("gumgum", 61)
    },
    AdapterSpec {
        required: &[&[&["pubId"]]],
        types: &[("pubId", ParamType::Text)],
        ad_system_domains: &["onetag.com"],
        ..spec("onetag", 241)
    },
    AdapterSpec {
        aliases: &["ra"],
        required: &[&[&["pid"]]],
        placement_param: Some("pid"),
        ad_system_domains: &["richaudience.com"],
        ..spec("richaudience", 108)
    },
    AdapterSpec {
        required: &[&[&["siteId"]]],
        ad_system_domains: &["video.unrulymedia.com", "rhythmone.com"],
        ..spec("unruly", 36)
    },
    AdapterSpec {
        required: &[&[&["placementId"]]],
        placement_param: Some("placementId"),
        ad_system_domains: &["kargo.com"],
        ..spec("kargo", 972)
    },
    AdapterSpec {
        aliases: &["33across_mgni"],
        required: &[&[&["zoneId"]], &[&["siteId"]]],
        types: &[
            ("zoneId", ParamType::Guid(22)),
            ("siteId", ParamType::Guid(22)),
        ],
        ad_system_domains: &["33across.com"],
        ..spec("33across", 58)
    },
];

/// The adapter behind a bidder code, by code or by one of its built-in
/// aliases. Aliases a publisher defines with `pbjs.aliasBidder` are unknown.
pub fn adapter_for(bidder: &str) -> Option<&'static AdapterSpec> {
    ADAPTERS
        .iter()
        .find(|adapter| adapter.code == bidder || adapter.aliases.contains(&bidder))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_adapter_the_brief_names_is_known_by_its_usual_code() {
        for bidder in [
            "appnexus",
            "rubicon",
            "ix",
            "pubmatic",
            "criteo",
            "openx",
            "triplelift",
            "sovrn",
            "seedtag",
            "adform",
            "smartadserver",
            "teads",
            "improvedigital",
            "sharethrough",
            "amx",
            "medianet",
            "ttd",
            "yieldmo",
            "gumgum",
            "onetag",
            "richaudience",
            "unruly",
            "kargo",
            "33across",
        ] {
            assert!(adapter_for(bidder).is_some(), "{bidder}");
        }
        assert_eq!(
            adapter_for("adform").map(|adapter| adapter.code),
            Some("adf")
        );
        assert_eq!(adapter_for("smart").map(|adapter| adapter.gvl_id), Some(45));
        assert_eq!(adapter_for("my-own-alias"), None);
    }

    #[test]
    fn codes_and_aliases_are_unique_and_the_table_is_consistent() {
        let mut names = BTreeSet::new();
        for adapter in ADAPTERS {
            for name in std::iter::once(&adapter.code).chain(adapter.aliases) {
                assert!(names.insert(*name), "{name} is listed twice");
            }
            let required_keys: BTreeSet<&str> = adapter
                .required
                .iter()
                .flat_map(|alternative| alternative.iter())
                .flat_map(|keys| keys.iter().copied())
                .collect();
            for key in adapter
                .placement_param
                .iter()
                .chain(adapter.account_param.iter())
            {
                assert!(
                    required_keys.contains(key),
                    "{}: {key} is not one of its required params",
                    adapter.code
                );
            }
            assert!(
                adapter.account_param.is_none() || !adapter.ad_system_domains.is_empty(),
                "{}: an account id needs an ad system to look it up in",
                adapter.code
            );
            for domain in adapter.ad_system_domains {
                assert_eq!(*domain, domain.to_ascii_lowercase());
            }
        }
    }
}
