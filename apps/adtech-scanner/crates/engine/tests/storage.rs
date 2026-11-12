//! The SQLite repositories against a real database.

use adscan_engine::application::ports::{LogRepository, ScanRepository, SettingsRepository};
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_model::{
    AdsTxtLineProblem, AdsTxtReport, AdsTxtStatus, BidderDiagnosis, BidderDiagnostics,
    ConsentReport, DeviceProfile, DiagnosticArea, DiagnosticCheck, DiagnosticStatus, Effort,
    Evidence, EvidenceKind, Finding, FindingCategory, FindingSource, LogEntry, LogKind, LogLevel,
    PipelineNode, PipelineStep, ScanListQuery, ScanMetrics, ScanOptions, ScanStatus, ScanSummary,
    Settings, Severity, SeverityCounts, StepStatus,
};
use serde_json::json;

fn summary(
    id: &str,
    url: &str,
    device: DeviceProfile,
    status: ScanStatus,
    started_at: f64,
) -> ScanSummary {
    ScanSummary {
        configuration: None,
        id: id.into(),
        url: url.into(),
        hostname: url
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .into(),
        device,
        status,
        started_at,
        finished_at: None,
        duration_ms: None,
        cost_usd: 0.0,
        finding_counts: SeverityCounts::default(),
        health_score: None,
        prebid_version: None,
        ai_used: false,
        error: None,
    }
}

fn options(url: &str) -> ScanOptions {
    ScanOptions {
        visitor: None,
        extra_pages: None,
        seed: None,
        headless: None,
        persistent_profile: None,
        proxy: None,
        budget_usd: None,
        url: url.into(),
        device: DeviceProfile::Desktop,
        observe_seconds: 20,
        auto_scroll: true,
        accept_consent: false,
        ai_enabled: true,
    }
}

fn finding(id: &str, scan_id: &str, severity: Severity) -> Finding {
    Finding {
        pages: Vec::new(),
        id: id.into(),
        scan_id: scan_id.into(),
        severity,
        category: FindingCategory::LogicBug,
        source: FindingSource::Ai,
        title: format!("finding {id}"),
        description: "d".into(),
        recommendation: "r".into(),
        confidence: 0.9,
        impact: "i".into(),
        estimated_revenue_impact_pct: Some(12.5),
        evidence: vec![Evidence {
            kind: EvidenceKind::Code,
            label: "check".into(),
            body: "return window.innerWidth > 768;".into(),
            script_url: Some("https://pub.example/ads.js".into()),
            line: Some(14),
        }],
        affected_slots: vec!["top".into()],
        affected_bidders: vec![],
    }
}

fn step(id: &str, scan_id: &str, node: PipelineNode, status: StepStatus) -> PipelineStep {
    PipelineStep {
        id: id.into(),
        scan_id: scan_id.into(),
        node,
        status,
        started_at: Some(1.0),
        duration_ms: None,
        summary: None,
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: 0.0,
        log_count: 0,
    }
}

fn log(id: &str, scan_id: &str, message: &str, data: Option<serde_json::Value>) -> LogEntry {
    LogEntry {
        id: id.into(),
        scan_id: scan_id.into(),
        step_id: "step-1".into(),
        node: PipelineNode::AnalyzeLogic,
        ts: 10.0,
        level: LogLevel::Info,
        kind: LogKind::Thinking,
        message: message.into(),
        data,
    }
}

async fn store_with_scan(id: &str) -> SqliteStore {
    let store = SqliteStore::open_in_memory().unwrap();
    create(
        &store,
        id,
        "https://pub.example/",
        DeviceProfile::Desktop,
        ScanStatus::Running,
        100.0,
    )
    .await;
    store
}

async fn create(
    store: &SqliteStore,
    id: &str,
    url: &str,
    device: DeviceProfile,
    status: ScanStatus,
    started_at: f64,
) {
    store
        .create(&summary(id, url, device, status, started_at), &options(url))
        .await
        .unwrap();
}

async fn count(store: &SqliteStore, scan_id: &str) -> (usize, usize) {
    let logs = LogRepository::list(store, scan_id, None)
        .await
        .unwrap()
        .len();
    let hits = store.search("innerWidth", 50).await.unwrap().len();
    (logs, hits)
}

#[tokio::test]
async fn scan_round_trips_through_summary_and_detail() {
    let store = store_with_scan("scan-1").await;
    assert_eq!(store.summary("missing").await.unwrap(), None);
    assert_eq!(store.detail("missing").await.unwrap(), None);

    let mut updated = summary(
        "scan-1",
        "https://pub.example/",
        DeviceProfile::Desktop,
        ScanStatus::Completed,
        100.0,
    );
    updated.finished_at = Some(900.0);
    updated.duration_ms = Some(800.0);
    updated.cost_usd = 0.31;
    updated.finding_counts = SeverityCounts {
        critical: 1,
        high: 2,
        medium: 0,
        low: 1,
        info: 0,
    };
    updated.health_score = Some(61);
    updated.prebid_version = Some("9.20.0".into());
    updated.ai_used = true;
    store.save_summary(&updated).await.unwrap();
    assert_eq!(
        store.summary("scan-1").await.unwrap(),
        Some(updated.clone())
    );

    let metrics = ScanMetrics {
        auction_count: 2,
        fill_rate: 0.5,
        has_prebid: true,
        ..ScanMetrics::default()
    };
    store
        .save_telemetry("scan-1", &metrics, &[], &[])
        .await
        .unwrap();
    store
        .save_findings(
            "scan-1",
            &[
                finding("f1", "scan-1", Severity::High),
                finding("f2", "scan-1", Severity::Low),
            ],
        )
        .await
        .unwrap();
    // Saving again replaces the list and keeps the given order.
    store
        .save_findings(
            "scan-1",
            &[
                finding("f2", "scan-1", Severity::Low),
                finding("f3", "scan-1", Severity::Critical),
            ],
        )
        .await
        .unwrap();
    store
        .save_overview("scan-1", "All about the scan.")
        .await
        .unwrap();
    let mut running = step(
        "step-1",
        "scan-1",
        PipelineNode::LaunchBrowser,
        StepStatus::Running,
    );
    store.save_step(&running).await.unwrap();
    store
        .save_step(&step(
            "step-2",
            "scan-1",
            PipelineNode::Capture,
            StepStatus::Pending,
        ))
        .await
        .unwrap();
    running.status = StepStatus::Completed;
    running.summary = Some("Chromium found".into());
    store.save_step(&running).await.unwrap();

    let detail = store.detail("scan-1").await.unwrap().unwrap();
    assert_eq!(detail.summary, updated);
    assert_eq!(detail.options, options("https://pub.example/"));
    assert_eq!(detail.metrics, metrics);
    assert_eq!(detail.overview.as_deref(), Some("All about the scan."));
    let ids: Vec<&str> = detail
        .findings
        .iter()
        .map(|finding| finding.id.as_str())
        .collect();
    assert_eq!(ids, ["f2", "f3"]);
    assert_eq!(
        detail.findings[1],
        finding("f3", "scan-1", Severity::Critical)
    );
    let steps: Vec<(PipelineNode, StepStatus)> = detail
        .steps
        .iter()
        .map(|step| (step.node, step.status))
        .collect();
    assert_eq!(
        steps,
        [
            (PipelineNode::LaunchBrowser, StepStatus::Completed),
            (PipelineNode::Capture, StepStatus::Pending)
        ]
    );

    // A scan has no bidder diagnostics until its checks ran; then they are
    // stored whole.
    assert_eq!(detail.bidder_diagnostics, None);
    let diagnostics = BidderDiagnostics {
        ads_txt: AdsTxtReport {
            url: "https://pub.example/ads.txt".into(),
            status: AdsTxtStatus::Found,
            detail: None,
            record_count: 12,
            invalid_line_count: 1,
            invalid_lines: vec![AdsTxtLineProblem {
                line: 7,
                excerpt: "google.com pub-1 DIRECT".into(),
                problem: "a record needs an ad system domain, an account id and DIRECT or RESELLER"
                    .into(),
            }],
            owner_domain: Some("pub-media.example".into()),
            manager_domains: vec!["manager.example".into()],
        },
        sellers_json_checked: true,
        consent: ConsentReport {
            cmp_present: true,
            consent_management_configured: Some(true),
            gdpr_applies: Some(true),
            resolved_at_ms: Some(812.5),
            auction_count: 2,
            auctions_before_consent: 1,
            consented_purposes: vec![1, 2, 7],
        },
        bidders: vec![BidderDiagnosis {
            bidder: "appnexus".into(),
            requests: 4,
            bids: 0,
            checks: vec![DiagnosticCheck {
                pages: Vec::new(),
                area: DiagnosticArea::AdsTxt,
                status: DiagnosticStatus::Problem,
                code: "ads-txt-not-listed".into(),
                summary: "ads.txt has no entry for appnexus.com".into(),
                excerpt: Some("xandr.example, 1, DIRECT".into()),
                ad_units: vec!["top".into()],
            }],
        }],
    };
    store
        .save_diagnostics("scan-1", &diagnostics)
        .await
        .unwrap();
    let detail = store.detail("scan-1").await.unwrap().unwrap();
    assert_eq!(detail.bidder_diagnostics, Some(diagnostics));
    assert_eq!(detail.overview.as_deref(), Some("All about the scan."));
}

#[tokio::test]
async fn list_filters_paginates_and_orders_newest_first() {
    let store = SqliteStore::open_in_memory().unwrap();
    create(
        &store,
        "a",
        "https://news.example/",
        DeviceProfile::Desktop,
        ScanStatus::Completed,
        100.0,
    )
    .await;
    create(
        &store,
        "b",
        "https://news.example/sport",
        DeviceProfile::Mobile,
        ScanStatus::Failed,
        200.0,
    )
    .await;
    create(
        &store,
        "c",
        "https://shop.example/",
        DeviceProfile::Mobile,
        ScanStatus::Completed,
        300.0,
    )
    .await;
    create(
        &store,
        "d",
        "https://100%_off.example/",
        DeviceProfile::Desktop,
        ScanStatus::Completed,
        400.0,
    )
    .await;

    let ids = |query: ScanListQuery| {
        let store = store.clone();
        async move {
            let list = ScanRepository::list(&store, &query).await.unwrap();
            (
                list.items
                    .into_iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>(),
                list.total,
            )
        }
    };
    let query = |search: Option<&str>, status, device, limit, offset| ScanListQuery {
        configuration_id: None,
        search: search.map(str::to_string),
        status,
        device,
        limit,
        offset,
    };

    assert_eq!(
        ids(query(None, None, None, None, None)).await,
        (vec!["d".into(), "c".into(), "b".into(), "a".into()], 4)
    );
    assert_eq!(
        ids(query(Some("NEWS"), None, None, None, None)).await,
        (vec!["b".into(), "a".into()], 2)
    );
    assert_eq!(
        ids(query(
            None,
            Some(ScanStatus::Completed),
            Some(DeviceProfile::Mobile),
            None,
            None
        ))
        .await,
        (vec!["c".into()], 1)
    );
    assert_eq!(
        ids(query(None, None, None, Some(2), Some(1))).await,
        (vec!["c".into(), "b".into()], 4)
    );
    assert_eq!(
        ids(query(None, None, None, Some(2), Some(10))).await,
        (vec![], 4)
    );
    // LIKE wildcards in the search text are literals.
    assert_eq!(
        ids(query(Some("%"), None, None, None, None)).await,
        (vec!["d".into()], 1)
    );
    assert_eq!(
        ids(query(Some("_off"), None, None, None, None)).await,
        (vec!["d".into()], 1)
    );
    assert_eq!(
        ids(query(
            Some("'; DROP TABLE scans; --"),
            None,
            None,
            None,
            None
        ))
        .await,
        (vec![], 0)
    );
    assert_eq!(ids(query(Some("  "), None, None, None, None)).await.1, 4);
}

#[tokio::test]
async fn deleting_a_scan_cascades_to_everything_including_the_search_index() {
    let store = store_with_scan("scan-1").await;
    create(
        &store,
        "scan-2",
        "https://other.example/",
        DeviceProfile::Desktop,
        ScanStatus::Completed,
        200.0,
    )
    .await;
    for scan in ["scan-1", "scan-2"] {
        store
            .save_findings(scan, &[finding(&format!("f-{scan}"), scan, Severity::High)])
            .await
            .unwrap();
        store
            .save_step(&step(
                &format!("step-{scan}"),
                scan,
                PipelineNode::Capture,
                StepStatus::Completed,
            ))
            .await
            .unwrap();
        store
            .append(&log(
                &format!("log-{scan}"),
                scan,
                "the check uses innerWidth",
                None,
            ))
            .await
            .unwrap();
    }
    assert_eq!(count(&store, "scan-1").await, (1, 2));

    assert!(store.delete("scan-1").await.unwrap());
    assert!(!store.delete("scan-1").await.unwrap());

    assert_eq!(store.detail("scan-1").await.unwrap(), None);
    assert_eq!(count(&store, "scan-1").await, (0, 1));
    assert_eq!(store.dashboard_stats().await.unwrap().total_findings, 1);
    let survivor = store.detail("scan-2").await.unwrap().unwrap();
    assert_eq!((survivor.findings.len(), survivor.steps.len()), (1, 1));

    store.delete_all().await.unwrap();
    assert_eq!(
        ScanRepository::list(
            &store,
            &ScanListQuery {
                configuration_id: None,
                search: None,
                status: None,
                device: None,
                limit: None,
                offset: None
            }
        )
        .await
        .unwrap()
        .total,
        0
    );
    assert_eq!(count(&store, "scan-2").await, (0, 0));
    assert_eq!(store.dashboard_stats().await.unwrap(), Default::default());
}

#[tokio::test]
async fn logs_are_listed_oldest_first_and_filtered_by_step() {
    let store = store_with_scan("scan-1").await;
    let mut other_step = log(
        "l2",
        "scan-1",
        "second",
        Some(json!({ "tool": "read_script" })),
    );
    other_step.step_id = "step-2".into();
    store
        .append(&log("l1", "scan-1", "first", None))
        .await
        .unwrap();
    store.append(&other_step).await.unwrap();
    store
        .append(&log("l3", "scan-1", "third", None))
        .await
        .unwrap();

    let all = LogRepository::list(&store, "scan-1", None).await.unwrap();
    assert_eq!(
        all.iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third"]
    );
    assert_eq!(all[1], other_step);
    let filtered = LogRepository::list(&store, "scan-1", Some("step-2"))
        .await
        .unwrap();
    assert_eq!(filtered, [other_step]);
}

#[tokio::test]
async fn search_finds_message_and_data_with_escaped_marked_snippets() {
    let store = store_with_scan("scan-1").await;
    store
        .append(&log(
            "l1",
            "scan-1",
            "The device check <b>isMobile</b> compares innerWidth > 768 & is inverted",
            None,
        ))
        .await
        .unwrap();
    store
        .append(&log(
            "l2",
            "scan-1",
            "read_script",
            Some(json!({ "input": { "script_id": "s3", "pattern": "requestBids" } })),
        ))
        .await
        .unwrap();

    let hits = store.search("inverted", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.id, "l1");
    assert_eq!(hits[0].scan_url, "https://pub.example/");
    assert_eq!(
        hits[0].snippet,
        "The device check &lt;b&gt;isMobile&lt;/b&gt; compares innerWidth &gt; 768 &amp; is <mark>inverted</mark>"
    );

    // Structured payloads are searchable, and the last term is a prefix match.
    let by_data = store.search("requestB", 10).await.unwrap();
    assert_eq!(
        by_data
            .iter()
            .map(|hit| hit.entry.id.as_str())
            .collect::<Vec<_>>(),
        ["l2"]
    );
    assert!(by_data[0].snippet.contains("<mark>requestBids</mark>"));
    // All terms must match.
    assert_eq!(store.search("device inverted", 10).await.unwrap().len(), 1);
    assert_eq!(
        store.search("device requestBids", 10).await.unwrap().len(),
        0
    );
    assert_eq!(store.search("inverted", 0).await.unwrap().len(), 0);
}

#[tokio::test]
async fn hostile_search_input_never_fails() {
    let store = store_with_scan("scan-1").await;
    store
        .append(&log(
            "l1",
            "scan-1",
            "pbjs requestBids near the auction",
            None,
        ))
        .await
        .unwrap();
    for query in [
        "",
        "   ",
        "\"",
        "\"unterminated",
        "*",
        "a*b",
        "NEAR(",
        "AND",
        "OR NOT",
        "pbjs AND",
        "col:value",
        "message:pbjs",
        "(((",
        "-pbjs",
        "^pbjs",
        "pbjs +",
        "'; DROP TABLE logs; --",
        "{a b}",
        "\\",
        "a\0b",
        "😀",
        "ß",
        "requestBids\"*",
    ] {
        let result = store.search(query, 10).await;
        assert!(result.is_ok(), "query {query:?} failed: {result:?}");
    }
    assert_eq!(
        store.search("pbjs OR zzz", 10).await.unwrap().len(),
        0,
        "operators are literal terms"
    );
    assert_eq!(store.search("-pbjs", 10).await.unwrap().len(), 1);
    assert_eq!(
        LogRepository::list(&store, "scan-1", None)
            .await
            .unwrap()
            .len(),
        1,
        "nothing was dropped"
    );
}

#[tokio::test]
async fn interrupted_scans_are_failed_on_recovery() {
    let store = SqliteStore::open_in_memory().unwrap();
    create(
        &store,
        "running",
        "https://a.example/",
        DeviceProfile::Desktop,
        ScanStatus::Running,
        1000.0,
    )
    .await;
    create(
        &store,
        "queued",
        "https://b.example/",
        DeviceProfile::Desktop,
        ScanStatus::Queued,
        2000.0,
    )
    .await;
    create(
        &store,
        "done",
        "https://c.example/",
        DeviceProfile::Desktop,
        ScanStatus::Completed,
        3000.0,
    )
    .await;
    store
        .save_step(&step(
            "s1",
            "running",
            PipelineNode::LaunchBrowser,
            StepStatus::Completed,
        ))
        .await
        .unwrap();
    store
        .save_step(&step(
            "s2",
            "running",
            PipelineNode::Capture,
            StepStatus::Running,
        ))
        .await
        .unwrap();
    store
        .save_step(&step(
            "s3",
            "running",
            PipelineNode::Telemetry,
            StepStatus::Pending,
        ))
        .await
        .unwrap();

    assert_eq!(
        store
            .fail_interrupted("The app was closed", 5000.0)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .fail_interrupted("The app was closed", 6000.0)
            .await
            .unwrap(),
        0
    );

    let running = store.detail("running").await.unwrap().unwrap();
    assert_eq!(running.summary.status, ScanStatus::Failed);
    assert_eq!(running.summary.error.as_deref(), Some("The app was closed"));
    assert_eq!(
        (running.summary.finished_at, running.summary.duration_ms),
        (Some(5000.0), Some(4000.0))
    );
    let statuses: Vec<StepStatus> = running.steps.iter().map(|step| step.status).collect();
    assert_eq!(
        statuses,
        [
            StepStatus::Completed,
            StepStatus::Failed,
            StepStatus::Skipped
        ]
    );
    assert_eq!(
        store.summary("queued").await.unwrap().unwrap().status,
        ScanStatus::Failed
    );
    let done = store.summary("done").await.unwrap().unwrap();
    assert_eq!((done.status, done.error), (ScanStatus::Completed, None));
}

#[tokio::test]
async fn dashboard_counts_open_criticals_from_the_latest_scan_per_page_and_device() {
    let store = SqliteStore::open_in_memory().unwrap();
    let completed =
        |id: &str, url: &str, device, started_at: f64, criticals: u32, cost: f64, score: u32| {
            let mut scan = summary(id, url, device, ScanStatus::Completed, started_at);
            scan.finding_counts.critical = criticals;
            scan.cost_usd = cost;
            scan.health_score = Some(score);
            scan
        };
    let scans = [
        completed(
            "old",
            "https://a.example/",
            DeviceProfile::Desktop,
            100.0,
            3,
            0.2,
            40,
        ),
        completed(
            "new",
            "https://a.example/",
            DeviceProfile::Desktop,
            200.0,
            1,
            0.4,
            70,
        ),
        completed(
            "mobile",
            "https://a.example/",
            DeviceProfile::Mobile,
            150.0,
            2,
            0.0,
            60,
        ),
        completed(
            "other",
            "https://b.example/",
            DeviceProfile::Desktop,
            300.0,
            0,
            0.2,
            100,
        ),
    ];
    for scan in &scans {
        store.create(scan, &options(&scan.url)).await.unwrap();
    }
    let mut failed = summary(
        "failed",
        "https://a.example/",
        DeviceProfile::Desktop,
        ScanStatus::Failed,
        400.0,
    );
    failed.finding_counts.critical = 9;
    store.create(&failed, &options(&failed.url)).await.unwrap();
    store
        .save_findings("new", &[finding("f1", "new", Severity::Critical)])
        .await
        .unwrap();

    let stats = store.dashboard_stats().await.unwrap();
    assert_eq!(stats.total_scans, 5);
    assert_eq!(stats.sites_scanned, 2);
    assert_eq!(
        stats.open_critical_findings, 3,
        "1 on desktop + 2 on mobile; superseded and failed scans do not count"
    );
    assert_eq!(stats.total_findings, 1);
    assert!((stats.total_cost_usd - 0.8).abs() < 1e-9);
    assert!((stats.avg_cost_usd - 0.16).abs() < 1e-9);
    assert_eq!(stats.avg_health_score, Some(67.5));
}

#[tokio::test]
async fn settings_persist_across_reopen_and_data_survives_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = SqliteStore::open(dir.path()).unwrap();
        assert_eq!(SettingsRepository::load(&store).await.unwrap(), None);
        let settings = Settings {
            analysis_effort: Effort::High,
            budget_usd: 1.25,
            chrome_path: Some("/opt/chrome".into()),
            check_sellers_json: true,
            ..Settings::default()
        };
        store.save(&settings).await.unwrap();
        store.save(&settings).await.unwrap();
        create(
            &store,
            "scan-1",
            "https://pub.example/",
            DeviceProfile::Desktop,
            ScanStatus::Completed,
            1.0,
        )
        .await;
    }
    let reopened = SqliteStore::open(dir.path()).unwrap();
    let settings = SettingsRepository::load(&reopened).await.unwrap().unwrap();
    assert_eq!(
        (settings.analysis_effort, settings.budget_usd),
        (Effort::High, 1.25)
    );
    assert_eq!(settings.chrome_path.as_deref(), Some("/opt/chrome"));
    assert!(settings.check_sellers_json);
    assert!(reopened.summary("scan-1").await.unwrap().is_some());
}
