// Use the Windows GUI subsystem in both debug installers and release builds.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if chatgpt_quota_monitor_lib::is_claude_relay(&args) {
        std::process::exit(chatgpt_quota_monitor_lib::claude_relay(&args));
    }
    if let Some(index) = args.iter().position(|a| a == "--claude-usage-smoke") {
        let path = args.get(index + 1).expect("scratch directory required");
        if let Err(error) = chatgpt_quota_monitor_lib::claude_usage_smoke(path) {
            eprintln!("CLAUDE_USAGE_SMOKE_FAIL: {error}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    if args.iter().any(|a| a == "--settings-ui-smoke") {
        chatgpt_quota_monitor_lib::settings_ui_smoke();
        return;
    }
    #[cfg(target_os = "macos")]
    if let Some(index) = args.iter().position(|a| a == "--quota-ui-smoke") {
        let path = args.get(index + 1).expect("PNG directory required");
        chatgpt_quota_monitor_lib::quota_ui_smoke(path);
        return;
    }
    chatgpt_quota_monitor_lib::run()
}
