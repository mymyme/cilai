use contexthub_application::{
    DurableEventReader, DurableEventRepository, NewDurableEvent, UnitOfWork,
};
use contexthub_domain::EventId;
use contexthub_persistence_sqlite::SqlitePersistence;
use contexthub_workspace_fs::{
    WorkspaceCancellationToken, WorkspaceScanOutcome, WorkspaceWatchSignal, WorkspaceWatcher,
    scan_workspace,
};
use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "contexthub-m0-benchmark-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&root)?;
    let result = benchmark(&root);
    fs::remove_dir_all(&root)?;
    result
}

fn benchmark(root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = root.join("workspace");
    fs::create_dir(&workspace)?;
    create_files(&workspace, 100)?;
    let small_scan_ms = scan(&workspace)?;
    create_files(&workspace, 10_000)?;
    let large_scan_ms = scan(&workspace)?;
    let watcher_ms = watcher_latency(&workspace)?;

    let database = root.join("events.db");
    let mut persistence = SqlitePersistence::open(&database, "0.1.0")?;
    let append_started = Instant::now();
    for index in 1..=1_000_i64 {
        persistence.transact(|repositories| {
            repositories
                .append_event(&NewDurableEvent {
                    event_id: EventId::new(format!("benchmark-event-{index}")),
                    workspace_id: None,
                    event_type: "benchmark.event".to_owned(),
                    object_type: Some("benchmark".to_owned()),
                    object_id: Some(index.to_string()),
                    payload_json: "{}".to_owned(),
                    correlation_id: Some("benchmark-correlation".to_owned()),
                    occurred_at: index,
                    created_at: index,
                })
                .map(|_| ())
        })?;
    }
    let append_ms = append_started.elapsed().as_millis();
    let replay_started = Instant::now();
    let mut cursor = 0_i64;
    let mut count = 0_usize;
    loop {
        let page = persistence.replay_global_after(cursor, 100)?;
        count += page.events.len();
        cursor = page.next_sequence;
        if !page.has_more {
            break;
        }
    }
    let replay_ms = replay_started.elapsed().as_millis();
    println!("scan_100_files_ms={small_scan_ms}");
    println!("scan_10000_files_ms={large_scan_ms}");
    println!("incremental_watch_latency_ms={watcher_ms}");
    println!("append_1000_events_ms={append_ms}");
    println!("replay_1000_events_ms={replay_ms}");
    println!("replayed_events={count}");
    if count != 1_000 {
        return Err("event replay count mismatch".into());
    }
    Ok(())
}

fn watcher_latency(root: &std::path::Path) -> Result<u128, Box<dyn std::error::Error>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _watcher = WorkspaceWatcher::start(root, move |signal| {
        let _ = sender.send(signal);
    })?;
    let started = Instant::now();
    fs::write(root.join("group-0/document-0.md"), "# Changed\n")?;
    loop {
        let signal = receiver.recv_timeout(std::time::Duration::from_secs(5))?;
        if matches!(signal, WorkspaceWatchSignal::Change(_)) {
            return Ok(started.elapsed().as_millis());
        }
    }
}

fn create_files(root: &std::path::Path, count: usize) -> Result<(), std::io::Error> {
    for index in 0..count {
        let directory = root.join(format!("group-{}", index / 100));
        fs::create_dir_all(&directory)?;
        let file = directory.join(format!("document-{index}.md"));
        if !file.exists() {
            fs::write(file, format!("# Document {index}\n\nBenchmark content.\n"))?;
        }
    }
    Ok(())
}

fn scan(root: &std::path::Path) -> Result<u128, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let outcome = scan_workspace(root, &WorkspaceCancellationToken::new(), |_| {})?;
    let WorkspaceScanOutcome::Completed(summary) = outcome else {
        return Err("benchmark scan cancelled".into());
    };
    if summary.issues != 0 {
        return Err("benchmark scan reported issues".into());
    }
    Ok(started.elapsed().as_millis())
}
