mod support;
use adscan_model::DeviceProfile;

#[tokio::test]
async fn explore() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let fixtures = support::Fixtures::start().await;
    for url in [site.url.clone(), fixtures.url("probe.html")] {
        let mut request = support::request(&url, DeviceProfile::Mobile);
        request.auto_scroll = true;
        request.observe_seconds = 8;
        let outcome = support::run(request).await;
        for line in &outcome.progress { println!("  > {line}"); }
        let capture = outcome.result.unwrap();
        for s in &capture.scripts { println!("  script {} inline={} init={:?} body={}", s.url, s.inline, s.initiator, &s.body[..s.body.len().min(50)].replace('\n', " ")); }
        for e in &capture.events { let j = serde_json::to_string(e).unwrap(); if !j.contains("prebidLog") && !j.contains("\"bid") { println!("  {}", &j[..j.len().min(200)]); } }
        for c in capture.console.iter() { if !c.text.contains("%cPrebid") {println!("  console {} {}", c.level, c.text); }}
    }
}
