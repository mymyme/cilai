use super::flush_memory_tasks;

#[tokio::test]
async fn flush_waits_for_pipeline_and_reports_its_failure() {
    let (release, pending) = tokio::sync::oneshot::channel::<()>();
    let mut tasks = vec![
        tokio::spawn(async { Err("extraction failed".to_string()) }),
        tokio::spawn(async move {
            pending.await.expect("pipeline released");
            Ok(())
        }),
    ];
    let wait = flush_memory_tasks(&mut tasks);
    tokio::pin!(wait);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut wait)
            .await
            .is_err()
    );
    release.send(()).expect("release pipeline");
    let error = wait.await.expect_err("pipeline failure must reach host");
    assert!(error.message.contains("extraction failed"));
}

#[tokio::test]
async fn flush_accepts_completed_and_empty_pipelines() {
    let mut tasks = vec![tokio::spawn(async { Ok(()) })];
    flush_memory_tasks(&mut tasks)
        .await
        .expect("completed pipeline");
    flush_memory_tasks(&mut []).await.expect("no pipeline");
}
