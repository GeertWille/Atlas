//! Human-readable progress lines for milestones in the tracker's event stream. No I/O.

use std::collections::BTreeSet;

use adscan_model::TrackerEventKind;

/// Remembers what has already been announced.
#[derive(Debug, Default)]
pub(crate) struct Milestones {
    auctions: u32,
    first_ad_announced: bool,
}

impl Milestones {
    /// The line to show for `event`, if it is worth one.
    pub(crate) fn announce(&mut self, event: &TrackerEventKind) -> Option<String> {
        match event {
            TrackerEventKind::PrebidDetected {
                global_name,
                version,
                ..
            } => Some(match version {
                Some(version) => format!("Prebid.js {version} detected (window.{global_name})"),
                None => format!("Prebid.js detected (window.{global_name})"),
            }),
            TrackerEventKind::GptDetected { .. } => Some("Google Publisher Tag detected".into()),
            TrackerEventKind::AmazonDetected {} => Some("Amazon TAM (apstag) detected".into()),
            TrackerEventKind::AuctionInit { ad_units, .. } => {
                self.auctions += 1;
                let bidders: BTreeSet<&str> = ad_units
                    .iter()
                    .flat_map(|unit| unit.bidders.iter().map(String::as_str))
                    .collect();
                // Ad units registered under one code are one ad unit.
                let codes: BTreeSet<&str> =
                    ad_units.iter().map(|unit| unit.code.as_str()).collect();
                Some(format!(
                    "Auction {} started with {} and {}",
                    self.auctions,
                    count(codes.len(), "ad unit"),
                    count(bidders.len(), "bidder"),
                ))
            }
            TrackerEventKind::SlotRenderEnded {
                element_id,
                is_empty: false,
                ..
            } if !self.first_ad_announced => {
                self.first_ad_announced = true;
                Some(format!("First ad rendered in #{element_id}"))
            }
            TrackerEventKind::PageLifecycle { phase } if phase == "firstScroll" => {
                Some("Page scrolled for the first time".into())
            }
            _ => None,
        }
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use adscan_model::TrackedAdUnit;

    use super::*;

    fn unit(code: &str, bidders: &[&str]) -> TrackedAdUnit {
        TrackedAdUnit {
            code: code.into(),
            sizes: vec![],
            bidders: bidders.iter().map(|bidder| bidder.to_string()).collect(),
            bids: vec![],
        }
    }

    #[test]
    fn auctions_are_numbered_and_count_distinct_bidders() {
        let mut milestones = Milestones::default();
        let first = TrackerEventKind::AuctionInit {
            auction_id: "a".into(),
            timeout: None,
            ad_units: vec![
                unit("top", &["appnexus", "rubicon"]),
                unit("side", &["rubicon"]),
            ],
        };
        let second = TrackerEventKind::AuctionInit {
            auction_id: "b".into(),
            timeout: None,
            ad_units: vec![unit("top", &["ix"]), unit("top", &["ix", "openx"])],
        };
        assert_eq!(
            milestones.announce(&first).unwrap(),
            "Auction 1 started with 2 ad units and 2 bidders"
        );
        assert_eq!(
            milestones.announce(&second).unwrap(),
            "Auction 2 started with 1 ad unit and 2 bidders",
            "two ad units registered under one code are one"
        );
    }

    #[test]
    fn only_the_first_filled_slot_is_announced() {
        let mut milestones = Milestones::default();
        let render = |element_id: &str, is_empty| TrackerEventKind::SlotRenderEnded {
            element_id: element_id.into(),
            is_empty,
            size: None,
            advertiser_id: None,
            line_item_id: None,
        };
        assert_eq!(milestones.announce(&render("ad-a", true)), None);
        assert_eq!(
            milestones.announce(&render("ad-b", false)).unwrap(),
            "First ad rendered in #ad-b"
        );
        assert_eq!(milestones.announce(&render("ad-c", false)), None);
    }

    #[test]
    fn library_detection_mentions_version_and_global() {
        let mut milestones = Milestones::default();
        let detected = TrackerEventKind::PrebidDetected {
            global_name: "acmeHb".into(),
            version: Some("v10.2.0".into()),
            modules: vec![],
        };
        assert_eq!(
            milestones.announce(&detected).unwrap(),
            "Prebid.js v10.2.0 detected (window.acmeHb)"
        );
        assert_eq!(
            milestones.announce(&TrackerEventKind::Scroll { depth: 0.2 }),
            None
        );
    }
}
