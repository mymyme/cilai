use contexthub_workbench_runtime::{ai_runtime::text_recovery, supervisor::AppServerSupervisor};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let [mode, profile, app_server, harness, document] = args.as_slice() else {
        return Err(
            "usage: history-recovery export|apply PROFILE APP_SERVER HARNESS DOCUMENT".into(),
        );
    };
    let profile = PathBuf::from(profile);
    let server = AppServerSupervisor::new();
    server.start(
        &PathBuf::from(app_server),
        "dev",
        &profile,
        &profile.join("run"),
    )?;
    let result = (|| match mode.as_str() {
        "export" => {
            let value = text_recovery::export(&server, &profile, &PathBuf::from(harness))?;
            std::fs::write(
                document,
                serde_json::to_vec(&value).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"exported":true}))
        }
        "apply" => {
            let plan = serde_json::from_slice(&std::fs::read(document).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            text_recovery::apply(&server, &profile, &PathBuf::from(harness), &plan)
        }
        _ => Err("history_recovery_mode_invalid".into()),
    })();
    server.shutdown()?;
    println!("{}", result?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
