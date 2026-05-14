use std::fs::OpenOptions;
use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

pub fn init_logging(debug: bool) {
    let filter = if debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    let log_path = log_file_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(file) => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_writer(move || file.try_clone().expect("log file clone failed"))
                .init();
        }
        Err(e) => {
            eprintln!(
                "Warning: could not open log file {:?}: {e}, logging to stderr",
                log_path
            );
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_writer(std::io::stderr)
                .init();
        }
    }
}

pub fn log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mekai")
        .join("logs")
}

pub fn log_file_path() -> PathBuf {
    log_dir().join("mekai.log")
}
