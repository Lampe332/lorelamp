use std::path::PathBuf;

use rp_server::{Config, Entry, Level, Logger, WebUi, DEFAULT_LLAMA_URL, DEFAULT_PORT};

const HELP: &str = "rp-server [--lan] [--port N] [--web-dir DIR] [--llama URL] [--data-dir DIR] [--manage-engine]\n\
  --lan        phones in the Wi-Fi may connect from the start, like the switch \"Handy\n\
               verbinden\" (env RP_LAN=1; own port and token, see \"phone:\" below)\n\
  --port N     default 7860 (env RP_PORT), 0 = any free port\n\
  --web-dir D  built web UI on disk (env RP_WEB_DIR); default: the one built into the\n\
               program (feature embed-ui), else apps/web/dist\n\
  --llama URL  llama-server, default http://127.0.0.1:8080 (env RP_LLAMA_URL)\n\
  --data-dir D database and cards, default %APPDATA%\\rp-app on Windows,\n\
               else the XDG data folder (env RP_DATA_DIR)\n\
  --manage-engine  start and supervise llama-server from the active model profile, like the\n\
               desktop app (env RP_MANAGE_ENGINE=1); --llama then follows the profile's port\n\
  env RP_LMSTUDIO_DIR  LM Studio's model folder for \"Meine Modelle\" (default\n\
               %USERPROFILE%\\.lmstudio\\models; empty = do not search it)\n\
  env RP_LAN_BIND  address of the phones' listener (default 0.0.0.0; test servers 127.0.0.1)\n\
  env RP_DISCOVERY_PORT  UDP port of the search in the Wi-Fi (default 47823)\n\
  env RP_GITHUB_API  source of the llama.cpp release list (test servers: a local stand-in)";

#[tokio::main]
async fn main() {
    let mut cfg = Config {
        port: std::env::var("RP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT),
        lan: std::env::var("RP_LAN").is_ok_and(|v| v == "1"),
        web: std::env::var("RP_WEB_DIR").map(|d| WebUi::Dir(d.into())).unwrap_or_else(|_| default_web()),
        llama_url: std::env::var("RP_LLAMA_URL").unwrap_or_else(|_| DEFAULT_LLAMA_URL.into()),
        data_dir: std::env::var("RP_DATA_DIR").map(PathBuf::from).unwrap_or_else(|_| rp_server::default_data_dir()),
        // Standalone: nobody here starts a model server, so say that while none answers.
        engine_note: std::sync::Arc::new(std::sync::Mutex::new(Some(rp_server::Note::new("standalone")))),
        log: None,
        manage_engine: std::env::var("RP_MANAGE_ENGINE").is_ok_and(|v| v == "1"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--lan" => cfg.lan = true,
            "--manage-engine" => cfg.manage_engine = true,
            "--port" => cfg.port = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| die("--port needs a number")),
            "--web-dir" => cfg.web = WebUi::Dir(args.next().map(PathBuf::from).unwrap_or_else(|| die("--web-dir needs a path"))),
            "--llama" => cfg.llama_url = args.next().unwrap_or_else(|| die("--llama needs a URL")),
            "--data-dir" => cfg.data_dir = args.next().map(PathBuf::from).unwrap_or_else(|| die("--data-dir needs a path")),
            "-h" | "--help" => {
                println!("{HELP}");
                return;
            }
            other => die(&format!("unknown argument {other}\n{HELP}")),
        }
    }

    if cfg.manage_engine {
        *cfg.engine_note.lock().unwrap_or_else(|p| p.into_inner()) = None; // this one does start the model
    }
    let log = Logger::start(&cfg.data_dir);
    cfg.log = Some(log.clone());
    let running = match rp_server::start(cfg).await {
        Ok(r) => r,
        Err(e) => die(&format!("cannot start server: {e}")),
    };
    println!("local:  {}", running.local_url());
    if running.lan {
        match running.lan_url() {
            Some(u) => println!("phone:  {u}"),
            None => println!("phone:  (no LAN IP found)"),
        }
    }
    let engine = running.engine.clone();
    tokio::select! {
        _ = running.wait() => {}
        _ = tokio::signal::ctrl_c() => log.write(Entry::new(Level::Info, "server", "stop", "server stopped (Ctrl+C)")),
    }
    if let Some(engine) = engine {
        engine.shutdown();
    }
    log.flush(std::time::Duration::from_secs(2));
}

/// Without `--web-dir`: the UI built into the program, else the build in the source tree.
fn default_web() -> WebUi {
    #[cfg(feature = "embed-ui")]
    return WebUi::Embedded;
    #[cfg(not(feature = "embed-ui"))]
    WebUi::Dir("apps/web/dist".into())
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2)
}
