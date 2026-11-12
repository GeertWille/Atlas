mod support;
use adscan_model::{DeviceProfile, TrackerEventKind as Kind, VisitorBehavior};

#[tokio::test]
#[ignore]
async fn explore() {
    let seeds: Vec<u32> = std::env::var("SEEDS").unwrap().split(',').map(|s| s.parse().unwrap()).collect();
    let device = if std::env::var("MOBILE").is_ok() { DeviceProfile::Mobile } else { DeviceProfile::Desktop };
    for seed in seeds {
        let site = adscan_browser::start_demo_site().await.unwrap();
        let mut request = support::request(&site.url, device);
        request.visitor = VisitorBehavior::Browse;
        request.extra_pages = 2;
        request.observe_seconds = 14;
        request.seed = seed;
        let outcome = support::run(request).await;
        let capture = outcome.result.unwrap();
        println!("== seed {seed}");
        for (i, page) in capture.pages.iter().enumerate() {
            let on = |f: &dyn Fn(&Kind) -> bool| capture.events.iter().filter(|e| e.page == i as u32 && f(&e.kind)).count();
            println!("  page {i} {:?} {} auctions={} requested={} rejected={} renderFailed={} depth={:.2}", page.reached_by, page.url.rsplit('/').next().unwrap(),
                on(&|k| matches!(k, Kind::AuctionInit{..})), on(&|k| matches!(k, Kind::SlotRequested{..})), on(&|k| matches!(k, Kind::SlotDefinitionRejected{..})), on(&|k| matches!(k, Kind::AdRenderFailed{..})), page.max_scroll_depth);
            for e in capture.events.iter().filter(|e| e.page == i as u32) {
                match &e.kind {
                    Kind::SlotRequested { element_id, prebid_ad_id, prebid_targeting_keys } => println!("     {:8.0} req {element_id} {:?} keys={}", e.t, prebid_ad_id, prebid_targeting_keys.len()),
                    Kind::AuctionInit { ad_units, .. } => println!("     {:8.0} auction {:?}", e.t, ad_units.iter().map(|u| u.code.as_str()).collect::<Vec<_>>()),
                    Kind::AdRenderFailed { ad_unit_code, reason, .. } => println!("     {:8.0} renderFailed {:?} {:?}", e.t, ad_unit_code, reason),
                    _ => {}
                }
            }
        }
        for line in capture.console.iter().filter(|l| l.text.starts_with("ENGAGEMENT")) { println!("  [{}] {}", line.page, line.text); }
        let hits = support::http_get(&format!("{}__demo/hits", site.url)).await;
        println!("  hits {hits}");
        for a in &outcome.activities { if matches!(a.kind, adscan_model::ActivityKind::LinkChoice | adscan_model::ActivityKind::Blocked) { println!("  {:?} {} {}", a.kind, a.message, a.data.as_ref().unwrap()["skipped"]); } }
    }
}
