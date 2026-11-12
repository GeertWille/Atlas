//! Reads a bid exchange as OpenRTB 2.x, with Prebid Server's `ext` fields.
//! Tolerant by design: bidders stretch the specification, so every field is
//! optional and a body that is not an OpenRTB object decodes to nothing.
//! Consent strings and user ids are reported as present, never copied.

use adscan_model::{
    DecodedBid, DecodedBidRequest, DecodedBidResponse, DecodedExchange, DecodedImp, DecodedSchain,
    DecodedSeat, ExchangeProtocol, SchainNode, ServerBidderError, ServerBidderTiming,
};
use serde_json::Value;

use super::classify::is_prebid_server;

/// `resolve` maps an id found in the exchange (an impression id, an ad unit
/// code) to the Prebid ad unit it stands for.
pub fn decode(
    path: &str,
    request_body: Option<&str>,
    response_body: Option<&str>,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Option<DecodedExchange> {
    let request_json = request_body.and_then(parse_object);
    let request = request_json.as_ref().and_then(|json| decode_request(json, resolve));
    let response = response_body
        .and_then(parse_object)
        .and_then(|json| decode_response(&json, resolve));
    if request.is_none() && response.is_none() {
        return None;
    }
    let names_server_bidders = request
        .as_ref()
        .is_some_and(|request| request.imps.iter().any(|imp| !imp.bidders.is_empty()));
    let has_server_ext = response.as_ref().is_some_and(|response| {
        !response.server_timings.is_empty() || !response.server_errors.is_empty()
    });
    let protocol = if is_prebid_server(path) || names_server_bidders || has_server_ext {
        ExchangeProtocol::PrebidServer
    } else {
        ExchangeProtocol::OpenRtb
    };
    Some(DecodedExchange {
        protocol,
        request,
        response,
    })
}

/// What OpenRTB calls a no-bid reason code (2.5 list 5.24, extended in 2.6).
pub fn nbr_meaning(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "Unknown Error",
        1 => "Technical Error",
        2 => "Invalid Request",
        3 => "Known Web Spider",
        4 => "Suspected Non-Human Traffic",
        5 => "Cloud, Data Center, or Proxy IP",
        6 => "Unsupported Device",
        7 => "Blocked Publisher or Site",
        8 => "Unmatched User",
        9 => "Daily Reader Cap Met",
        10 => "Daily Domain Cap Met",
        11 => "Ads.txt Authorization Unavailable",
        12 => "Ads.txt Authorization Violation",
        13 => "Ads.cert Authentication Unavailable",
        14 => "Ads.cert Authentication Violation",
        15 => "Insufficient Auction Time",
        16 => "Incomplete SupplyChain",
        17 => "Blocked SupplyChain Node",
        _ => return None,
    })
}

fn parse_object(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
}

fn decode_request(json: &Value, resolve: &dyn Fn(&str) -> Option<String>) -> Option<DecodedBidRequest> {
    let imps = json.get("imp")?.as_array()?;
    let user = &json["user"];
    let regs = &json["regs"];
    let source = &json["source"];
    let schain = either(&source["ext"]["schain"], &source["schain"]);
    Some(DecodedBidRequest {
        id: text(&json["id"]),
        tmax_ms: json["tmax"].as_f64(),
        imps: imps.iter().map(|imp| decode_imp(imp, resolve)).collect(),
        site_domain: text(&json["site"]["domain"]),
        site_page: text(&json["site"]["page"]),
        currencies: strings(&json["cur"]),
        schain: schain.as_object().map(|_| DecodedSchain {
            complete: flag(&schain["complete"]),
            nodes: schain["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|node| {
                    Some(SchainNode {
                        asi: text(&node["asi"])?,
                        sid: text(&node["sid"])?,
                        hp: flag(&node["hp"]),
                    })
                })
                .collect(),
        }),
        gdpr: flag(either(&regs["ext"]["gdpr"], &regs["gdpr"])),
        coppa: flag(&regs["coppa"]),
        consent_string_present: present(either(&user["ext"]["consent"], &user["consent"])),
        us_privacy_present: present(either(&regs["ext"]["us_privacy"], &regs["us_privacy"])),
        gpp_present: present(either(&regs["gpp"], &regs["ext"]["gpp"])),
        eid_sources: either(&user["ext"]["eids"], &user["eids"])
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|eid| text(&eid["source"]))
            .collect(),
        test: flag(&json["test"]).unwrap_or(false),
    })
}

fn decode_imp(imp: &Value, resolve: &dyn Fn(&str) -> Option<String>) -> DecodedImp {
    let id = text(&imp["id"]).unwrap_or_default();
    let tag_id = text(&imp["tagid"]);
    let named_unit = text(&imp["ext"]["prebid"]["adunitcode"]);
    let ad_unit = resolve(&id)
        .or_else(|| named_unit.as_deref().and_then(resolve).or(named_unit))
        .or_else(|| tag_id.as_deref().and_then(resolve));
    let mut sizes = Vec::new();
    let mut media_types = Vec::new();
    for media in ["banner", "video", "native", "audio"] {
        let Some(object) = imp[media].as_object() else {
            continue;
        };
        media_types.push(media.to_string());
        for format in object.get("format").and_then(Value::as_array).into_iter().flatten() {
            sizes.extend(size(&format["w"], &format["h"]));
        }
        sizes.extend(size(&imp[media]["w"], &imp[media]["h"]));
    }
    sizes.dedup();
    DecodedImp {
        id,
        ad_unit,
        tag_id,
        media_types,
        sizes,
        floor: imp["bidfloor"].as_f64().filter(|floor| *floor > 0.0),
        floor_currency: text(&imp["bidfloorcur"]),
        bidders: imp["ext"]["prebid"]["bidder"]
            .as_object()
            .map(|bidders| bidders.keys().cloned().collect())
            .unwrap_or_default(),
    }
}

fn decode_response(json: &Value, resolve: &dyn Fn(&str) -> Option<String>) -> Option<DecodedBidResponse> {
    let seat_bids = json.get("seatbid").and_then(Value::as_array);
    let nbr = json.get("nbr").and_then(Value::as_u64).map(|code| code as u32);
    // An object that says neither is some other JSON, not a bid response.
    if seat_bids.is_none() && nbr.is_none() {
        return None;
    }
    let ext = &json["ext"];
    Some(DecodedBidResponse {
        id: text(&json["id"]),
        currency: text(&json["cur"]),
        nbr,
        nbr_meaning: nbr.and_then(nbr_meaning).map(str::to_string),
        seats: seat_bids
            .into_iter()
            .flatten()
            .map(|seat| DecodedSeat {
                seat: text(&seat["seat"]),
                bids: seat["bid"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|bid| {
                        let imp_id = text(&bid["impid"]).unwrap_or_default();
                        DecodedBid {
                            ad_unit: resolve(&imp_id),
                            imp_id,
                            price: bid["price"].as_f64().unwrap_or(0.0),
                            size: size(&bid["w"], &bid["h"]),
                            advertiser_domains: strings(&bid["adomain"]),
                            creative_id: text(&bid["crid"]),
                            deal_id: text(&bid["dealid"]),
                            markup_bytes: bid["adm"].as_str().map(|markup| markup.len() as f64),
                        }
                    })
                    .collect(),
            })
            .collect(),
        server_timings: ext["responsetimemillis"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(bidder, ms)| {
                Some(ServerBidderTiming {
                    bidder: bidder.clone(),
                    ms: ms.as_f64()?,
                })
            })
            .collect(),
        server_errors: ext["errors"]
            .as_object()
            .into_iter()
            .flatten()
            .flat_map(|(bidder, errors)| {
                errors.as_array().into_iter().flatten().map(move |error| ServerBidderError {
                    bidder: bidder.clone(),
                    code: error["code"].as_u64().map(|code| code as u32),
                    message: text(&error["message"]).unwrap_or_default(),
                })
            })
            .collect(),
    })
}

/// The highest price a decoded response carries.
pub fn best_price(response: &DecodedBidResponse) -> Option<f64> {
    response
        .seats
        .iter()
        .flat_map(|seat| &seat.bids)
        .map(|bid| bid.price)
        .fold(None, |best, price| Some(best.map_or(price, |best: f64| best.max(price))))
}

fn either<'a>(first: &'a Value, second: &'a Value) -> &'a Value {
    if first.is_null() { second } else { first }
}

/// Ids are strings in the specification and numbers in the wild.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// OpenRTB flags are 0 or 1; some senders use booleans.
fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => number.as_f64().map(|number| number != 0.0),
        _ => None,
    }
}

fn present(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Null => false,
        _ => true,
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(text)
        .collect()
}

fn size(width: &Value, height: &Value) -> Option<String> {
    Some(format!("{}x{}", width.as_u64()?, height.as_u64()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_units(_: &str) -> Option<String> {
        None
    }

    const OPENRTB_REQUEST: &str = r#"{
        "id": "req-7f3a", "tmax": 1200, "cur": ["USD"], "test": 0,
        "imp": [
            {"id": "2a", "tagid": "top-placement", "bidfloor": 0.25, "bidfloorcur": "USD",
             "banner": {"format": [{"w": 970, "h": 250}, {"w": 728, "h": 90}]}},
            {"id": 3, "video": {"w": 640, "h": 360}}
        ],
        "site": {"domain": "pub.example", "page": "https://pub.example/a"},
        "user": {"ext": {"consent": "CPXXXX-not-shown", "eids": [
            {"source": "id5-sync.com", "uids": [{"id": "ID5-secret"}]},
            {"source": "liveramp.com", "uids": [{"id": "RL-secret"}]}]}},
        "regs": {"coppa": 0, "ext": {"gdpr": 1, "us_privacy": "1YNN"}},
        "source": {"ext": {"schain": {"complete": 1, "ver": "1.0", "nodes": [
            {"asi": "pub.example", "sid": "1234", "hp": 1}]}}}
    }"#;

    #[test]
    fn an_openrtb_request_decodes_to_impressions_supply_chain_and_flags() {
        let units = |id: &str| (id == "2a").then(|| "top".to_string());
        let exchange = decode("/openrtb/bid", Some(OPENRTB_REQUEST), None, &units).unwrap();
        assert_eq!(exchange.protocol, ExchangeProtocol::OpenRtb);
        assert_eq!(exchange.response, None);
        let request = exchange.request.unwrap();
        assert_eq!(request.id.as_deref(), Some("req-7f3a"));
        assert_eq!(request.tmax_ms, Some(1200.0));
        assert_eq!(request.currencies, ["USD"]);
        assert!(!request.test);
        assert_eq!(request.site_domain.as_deref(), Some("pub.example"));
        assert_eq!(
            request.imps[0],
            DecodedImp {
                id: "2a".into(),
                ad_unit: Some("top".into()),
                tag_id: Some("top-placement".into()),
                media_types: vec!["banner".into()],
                sizes: vec!["970x250".into(), "728x90".into()],
                floor: Some(0.25),
                floor_currency: Some("USD".into()),
                bidders: Vec::new(),
            }
        );
        assert_eq!(request.imps[1].id, "3", "a numeric id reads as text");
        assert_eq!(request.imps[1].sizes, ["640x360"]);
        assert_eq!(request.imps[1].floor, None);
        assert_eq!((request.gdpr, request.coppa), (Some(true), Some(false)));
        assert!(request.consent_string_present && request.us_privacy_present);
        assert!(!request.gpp_present);
        assert_eq!(request.eid_sources, ["id5-sync.com", "liveramp.com"]);
        let schain = request.schain.unwrap();
        assert_eq!(schain.complete, Some(true));
        assert_eq!(
            schain.nodes,
            [SchainNode {
                asi: "pub.example".into(),
                sid: "1234".into(),
                hp: Some(true)
            }]
        );
    }

    #[test]
    fn the_decoded_request_never_carries_the_consent_string_or_the_user_ids() {
        let exchange = decode("/x", Some(OPENRTB_REQUEST), None, &no_units).unwrap();
        let rendered = serde_json::to_string(&exchange).unwrap();
        for secret in ["CPXXXX-not-shown", "ID5-secret", "RL-secret", "1YNN"] {
            assert!(!rendered.contains(secret), "{secret}");
        }
    }

    #[test]
    fn openrtb_2_6_puts_regs_schain_and_consent_at_the_top_level() {
        let body = r#"{"id":"1","imp":[{"id":"1","banner":{"w":300,"h":250}}],
            "regs":{"gdpr":0,"gpp":"DBABMA~","us_privacy":"1---"},
            "user":{"consent":"CP","eids":[{"source":"uidapi.com"}]},
            "source":{"schain":{"complete":0,"nodes":[]}}}"#;
        let request = decode("/x", Some(body), None, &no_units).unwrap().request.unwrap();
        assert_eq!(request.gdpr, Some(false));
        assert!(request.gpp_present && request.us_privacy_present && request.consent_string_present);
        assert_eq!(request.eid_sources, ["uidapi.com"]);
        assert_eq!(request.schain.unwrap().complete, Some(false));
        assert_eq!(request.imps[0].sizes, ["300x250"]);
    }

    #[test]
    fn a_response_with_bids_decodes_to_seats() {
        let body = r#"{"id":"req-7f3a","cur":"USD","seatbid":[{"seat":"pubmatic","bid":[
            {"impid":"2a","price":1.42,"w":970,"h":250,"adomain":["acme.example"],"crid":"cr-88121",
             "adm":"<div>creative</div>"},
            {"impid":"2c","price":0.61,"w":320,"h":50,"adomain":["shoes.example"],"crid":"cr-1077",
             "dealid":"PM-DEAL9"}]}]}"#;
        let units = |id: &str| match id {
            "2a" => Some("top".to_string()),
            "2c" => Some("anchor".to_string()),
            _ => None,
        };
        let response = decode("/x", None, Some(body), &units).unwrap().response.unwrap();
        assert_eq!(response.currency.as_deref(), Some("USD"));
        assert_eq!((response.nbr, response.nbr_meaning.as_deref()), (None, None));
        let bids = &response.seats[0].bids;
        assert_eq!(response.seats[0].seat.as_deref(), Some("pubmatic"));
        assert_eq!(
            bids[0],
            DecodedBid {
                imp_id: "2a".into(),
                ad_unit: Some("top".into()),
                price: 1.42,
                size: Some("970x250".into()),
                advertiser_domains: vec!["acme.example".into()],
                creative_id: Some("cr-88121".into()),
                deal_id: None,
                markup_bytes: Some(19.0),
            }
        );
        assert_eq!(bids[1].deal_id.as_deref(), Some("PM-DEAL9"));
        assert_eq!(bids[1].markup_bytes, None);
        assert_eq!(best_price(&response), Some(1.42));
    }

    #[test]
    fn a_no_bid_reason_is_decoded_to_words() {
        let response = decode("/openrtbb/prebidjs", None, Some(r#"{"id":"meridian-post","nbr":7}"#), &no_units)
            .unwrap()
            .response
            .unwrap();
        assert_eq!(response.nbr, Some(7));
        assert_eq!(response.nbr_meaning.as_deref(), Some("Blocked Publisher or Site"));
        assert!(response.seats.is_empty());
        assert_eq!(best_price(&response), None);
    }

    #[test]
    fn every_listed_no_bid_reason_has_a_name_and_others_have_none() {
        for code in 0..=17 {
            assert!(nbr_meaning(code).is_some(), "{code}");
        }
        assert_eq!(nbr_meaning(2), Some("Invalid Request"));
        assert_eq!(nbr_meaning(18), None);
        assert_eq!(nbr_meaning(500), None, "exchange-specific codes are left as numbers");
    }

    #[test]
    fn prebid_server_names_its_bidders_their_timings_and_their_errors() {
        let request = r#"{"id":"auction-1","tmax":1000,"imp":[
            {"id":"top","banner":{"format":[{"w":970,"h":250}]},
             "ext":{"prebid":{"adunitcode":"top","bidder":{"appnexus":{"placementId":1},"sovrn":{"tagid":"x"}}}}}]}"#;
        let response = r#"{"id":"auction-1","seatbid":[{"seat":"appnexus","bid":[{"impid":"top","price":0.88}]}],
            "ext":{"responsetimemillis":{"appnexus":140,"sovrn":1000},
                   "errors":{"sovrn":[{"code":1,"message":"timeout"}]}}}"#;
        let known = |id: &str| (id == "top").then(|| "top".to_string());
        let exchange = decode("/openrtb2/auction", Some(request), Some(response), &known).unwrap();
        assert_eq!(exchange.protocol, ExchangeProtocol::PrebidServer);
        let imp = &exchange.request.unwrap().imps[0];
        assert_eq!(imp.ad_unit.as_deref(), Some("top"));
        assert_eq!(imp.bidders, ["appnexus", "sovrn"]);
        let response = exchange.response.unwrap();
        assert_eq!(
            response.server_timings,
            [
                ServerBidderTiming { bidder: "appnexus".into(), ms: 140.0 },
                ServerBidderTiming { bidder: "sovrn".into(), ms: 1000.0 },
            ]
        );
        assert_eq!(
            response.server_errors,
            [ServerBidderError { bidder: "sovrn".into(), code: Some(1), message: "timeout".into() }]
        );
        assert_eq!(response.seats[0].bids[0].ad_unit.as_deref(), Some("top"));
    }

    #[test]
    fn a_prebid_server_request_on_another_path_is_recognised_by_its_ext() {
        let request = r#"{"id":"1","imp":[{"id":"a","ext":{"prebid":{"bidder":{"ix":{}}}}}]}"#;
        let exchange = decode("/custom/auction", Some(request), None, &no_units).unwrap();
        assert_eq!(exchange.protocol, ExchangeProtocol::PrebidServer);
    }

    #[test]
    fn bodies_that_are_not_openrtb_decode_to_nothing() {
        for (request, response) in [
            (None, None),
            (Some("not json"), Some("<html></html>")),
            (Some("[1,2,3]"), Some("42")),
            (Some(r#"{"tags":[{"uuid":"1"}]}"#), Some(r#"{"tags":[{"error":"x"}]}"#)),
            (Some(""), Some("")),
            (None, Some(r#"{"status":"ok"}"#)),
        ] {
            assert_eq!(decode("/ut/v3/prebid", request, response, &no_units), None, "{request:?}");
        }
    }

    #[test]
    fn one_readable_side_is_enough() {
        let exchange = decode("/x", Some("garbage"), Some(r#"{"seatbid":[]}"#), &no_units).unwrap();
        assert_eq!(exchange.request, None);
        assert!(exchange.response.unwrap().seats.is_empty());
    }
}
