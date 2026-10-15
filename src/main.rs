use clap::Parser;
use futures_util::future::try_join_all;
use karlsen_alloc::init_allocator_with_default_settings;
use karlsen_stratum_bridge::log_colors::LogColors;
use karlsen_stratum_bridge::{KarlsenApi, StratumServerBridgeConfig as StratumBridgeConfig, listen_and_serve_with_shutdown, prom};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[cfg(windows)]
use windows_sys::Win32::System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler};

mod app_dirs;
mod cli;
mod health_check;
mod tracing_setup;

use cli::{Cli, apply_cli_overrides};
use karlsen_stratum_bridge::BridgeConfig;

static CONFIG_LOADED_FROM: OnceLock<Option<PathBuf>> = OnceLock::new();
static REQUESTED_CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

#[cfg(windows)]
struct CtrlHandlerState {
    presses: AtomicUsize,
    last_event_ms: AtomicU64,
    shutdown_tx: watch::Sender<bool>,
}

#[cfg(windows)]
static CTRL_HANDLER_STATE: OnceLock<CtrlHandlerState> = OnceLock::new();

#[cfg(windows)]
unsafe extern "system" fn console_ctrl_handler(ctrl_type: u32) -> i32 {
    if ctrl_type != CTRL_C_EVENT {
        return 0;
    }

    let Some(state) = CTRL_HANDLER_STATE.get() else {
        return 0;
    };

    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);

    // Debounce: Windows can invoke the handler multiple times for a single keypress.
    // Treat multiple invocations within a short window as the same press.
    const DEBOUNCE_MS: u64 = 500;

    let last_ms = state.last_event_ms.load(Ordering::SeqCst);
    if last_ms != 0 && now_ms.saturating_sub(last_ms) < DEBOUNCE_MS {
        return 1;
    }
    state.last_event_ms.store(now_ms, Ordering::SeqCst);

    let prev = state.presses.fetch_add(1, Ordering::SeqCst);
    if prev == 0 {
        let _ = state.shutdown_tx.send(true);
        1
    } else {
        std::process::exit(130);
    }
}

#[cfg(windows)]
fn install_windows_ctrl_handler(shutdown_tx: watch::Sender<bool>) -> Result<(), anyhow::Error> {
    let _ = CTRL_HANDLER_STATE.set(CtrlHandlerState { presses: AtomicUsize::new(0), last_event_ms: AtomicU64::new(0), shutdown_tx });

    let ok = unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), 1) };
    if ok == 0 {
        return Err(anyhow::anyhow!("failed to install Windows console control handler"));
    }
    Ok(())
}

fn initialize_config() -> BridgeConfig {
    let config_path = REQUESTED_CONFIG_PATH.get().map(PathBuf::as_path).unwrap_or_else(|| Path::new("config.yaml"));
    // Load config first to check if file logging is enabled
    let fallback_path = Path::new("bridge").join(config_path);
    // Build candidate paths for config file search:
    // 1. Direct path as specified
    // 2. Fallback path under ./bridge/
    // 3-5. Paths relative to executable directory (for different deployment scenarios)
    let exe_base = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let exe_root = exe_base.as_ref().and_then(|p| p.parent()).and_then(|p| p.parent()).map(|p| p.to_path_buf());

    let mut candidates: Vec<std::path::PathBuf> = vec![config_path.to_path_buf(), fallback_path.clone()];

    if config_path.is_relative() {
        if let Some(exe_base) = exe_base.as_ref() {
            candidates.push(exe_base.join(config_path));
        }
        if let Some(exe_root) = exe_root.as_ref() {
            candidates.push(exe_root.join(config_path));
            candidates.push(exe_root.join("bridge").join(config_path));
        }
    }

    let mut loaded_from: Option<std::path::PathBuf> = None;
    let mut config: Option<BridgeConfig> = None;
    for path in candidates.iter() {
        if path.exists() {
            let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("Failed to read config file {}: {}", path.display(), e);
                std::process::exit(1);
            });

            let parsed = BridgeConfig::from_yaml(&content).unwrap_or_else(|e| {
                eprintln!("Failed to parse config file {}: {}", path.display(), e);
                std::process::exit(1);
            });

            config = Some(parsed);
            loaded_from = Some(path.clone());
            break;
        }
    }

    if CONFIG_LOADED_FROM.set(loaded_from).is_err() {
        tracing::warn!("Failed to set config loaded from path - may already be initialized");
    }
    config.unwrap_or_default()
}

/// Log the bridge configuration at startup
fn log_bridge_configuration(config: &BridgeConfig) {
    let instance_count = config.instances.len();
    tracing::info!("----------------------------------");
    tracing::info!("initializing bridge ({} instance{})", instance_count, if instance_count > 1 { "s" } else { "" });
    tracing::info!("\tkarlsend:        {} (shared)", config.global.karlsend_address);
    tracing::info!("\tblock wait:      {:?}", config.global.block_wait_time);
    tracing::info!("\tprint stats:     {}", config.global.print_stats);
    tracing::info!("\tvar diff:        {}", config.global.var_diff);
    tracing::info!("\tsolo mining:     {}", config.global.solo_mining);
    tracing::info!("\ttestnet mining:  {}", config.global.testnet_mining);
    tracing::info!("\tshares per min:  {}", config.global.shares_per_min);
    tracing::info!("\tvar diff stats:  {}", config.global.var_diff_stats);
    tracing::info!("\tpow2 clamp:      {}", config.global.pow2_clamp);
    tracing::info!("\textranonce:      auto-detected per client");
    tracing::info!("\thealth check:    {}", config.global.health_check_port);

    for (idx, instance) in config.instances.iter().enumerate() {
        #[allow(clippy::arithmetic_side_effects, reason = "ARITH-SAFETY(INDEX)")]
        {
            tracing::info!("\t--- Instance {} ---", idx + 1);
        }
        tracing::info!("\t  stratum:       {}", instance.stratum_port);
        tracing::info!("\t  min diff:      {}", instance.min_share_diff);
        if let Some(ref prom_port) = instance.prom_port {
            tracing::info!("\t  prom:          {}", prom_port);
        }
        if let Some(log_to_file) = instance.log_to_file {
            tracing::info!("\t  log to file:   {}", log_to_file);
        }
    }
    tracing::info!("----------------------------------");
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    init_allocator_with_default_settings();

    let cli = Cli::parse();

    // Single-config model: default to `config.yaml` for both mainnet and testnet runs.
    // `--testnet` affects the network behavior, but does not imply a different config file.
    let requested_config = cli.config.clone().unwrap_or_else(|| PathBuf::from("config.yaml"));

    if REQUESTED_CONFIG_PATH.set(requested_config.clone()).is_err() {
        tracing::warn!("Failed to set requested config path - may already be initialized");
    }

    let mut config = initialize_config();
    apply_cli_overrides(&mut config, &cli)?;

    // Initialize color support detection
    LogColors::init();

    // Provide web/prom status endpoints with the *actual* effective config (after CLI overrides),
    // instead of having the server re-read `config.yaml` from disk.
    // This is best-effort and does not affect any mining logic.
    prom::set_web_status_config(config.global.karlsend_address.clone(), config.instances.len());
    // Point the web config endpoint at the actual config file path the bridge is using.
    let loaded_config_path = CONFIG_LOADED_FROM.get().and_then(|p| p.as_ref()).cloned().unwrap_or_else(|| requested_config.clone());
    prom::set_web_config_path(loaded_config_path);

    // Initialize tracing with WARN level by default (less verbose)
    // Can be overridden with RUST_LOG environment variable (e.g., RUST_LOG=info,debug)
    // To see more details, set RUST_LOG=info or RUST_LOG=debug
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,karlsen_stratum_bridge=info"));

    // Note: The file_guard must be kept alive for the lifetime of the program
    // to ensure logs are flushed to the file
    // Store it in a OnceLock to prevent it from being dropped
    static FILE_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> = std::sync::OnceLock::new();
    if let Some(guard) = tracing_setup::init_tracing(&config, filter, false) {
        let _ = FILE_GUARD.set(guard);
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Install our own console handler so Ctrl+C triggers graceful shutdown.
    #[cfg(windows)]
    install_windows_ctrl_handler(shutdown_tx.clone())?;

    if CONFIG_LOADED_FROM.get().and_then(|p| p.as_ref()).is_none() {
        let config_path = requested_config.as_path();
        let cwd = std::env::current_dir().ok();
        tracing::warn!("config.yaml not found, using defaults (requested: {:?}, cwd: {:?})", config_path, cwd);
    }

    log_bridge_configuration(&config);

    // Start global health check server if port is specified
    if !config.global.health_check_port.is_empty() {
        let health_port = config.global.health_check_port.clone();
        health_check::spawn_health_check_server(health_port);
    }

    // Create shared karlsen API client (all instances use the same node)
    let karlsen_api =
        KarlsenApi::new(config.global.karlsend_address.clone(), config.global.coinbase_tag_suffix.clone(), shutdown_rx.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create Karlsen API client: {}", e))?;

    if !config.global.web_dashboard_port.is_empty() {
        let web_dashboard_port = config.global.web_dashboard_port.clone();
        tokio::spawn(async move {
            if let Err(e) = prom::start_web_server_all(&web_dashboard_port).await {
                tracing::error!("Aggregated web server error: {}", e);
            }
        });
    }

    karlsen_stratum_bridge::share_handler::set_vardiff_min_diff(config.global.vardiff_min_diff);
    karlsen_stratum_bridge::client_handler::set_solo_mining(config.global.solo_mining);
    karlsen_stratum_bridge::default_client::set_testnet_mining(config.global.testnet_mining);
    if config.global.solo_mining {
        tracing::info!("Solo mining enabled: miners receive the network difficulty, vardiff is disabled");
    }

    // Build the FishHash context before listeners start so the first share does not pay for it.
    {
        let full_dataset = config.global.full_dataset;
        tokio::task::spawn_blocking(move || karlsen_stratum_bridge::pow_context::init_fish_context(full_dataset))
            .await
            .map_err(|e| anyhow::anyhow!("FishHash context init failed: {}", e))?;
    }

    tracing::info!("Waiting for node to fully sync before starting stratum listeners");
    karlsen_api
        .wait_for_sync_with_shutdown(shutdown_rx.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed while waiting for node sync: {}", e))?;
    tracing::info!("Node is synced, starting stratum listeners");

    let mut instance_handles = Vec::new();
    for (idx, instance_config) in config.instances.iter().enumerate() {
        #[allow(clippy::arithmetic_side_effects, reason = "ARITH-SAFETY(INDEX)")]
        let instance_num = idx + 1;
        let instance = instance_config.clone();
        let global = config.global.clone();
        let karlsen_api_clone = Arc::clone(&karlsen_api);
        let instance_shutdown_rx = shutdown_rx.clone();

        let is_first_instance = idx == 0;

        let instance_id_str = LogColors::format_instance_id(instance_num);

        if let Some(ref prom_port) = instance.prom_port {
            let prom_port = prom_port.clone();
            let instance_num_prom = instance_num;
            let instance_id_prom = instance_id_str.clone();
            tokio::spawn(async move {
                if let Err(e) = prom::start_prom_server(&prom_port, &instance_id_prom).await {
                    tracing::error!("[Instance {}] Prometheus server error: {}", instance_num_prom, e);
                }
            });
        }

        let handle = tokio::spawn(async move {
            tracing_setup::register_instance(instance_id_str.clone(), instance_num);

            let colored_instance_id = LogColors::format_instance_id(instance_num);
            tracing::info!("{} Starting on stratum port {}", colored_instance_id, instance.stratum_port);

            let bridge_config = StratumBridgeConfig {
                instance_id: instance_id_str.clone(),
                stratum_port: instance.stratum_port.clone(),
                karlsend_address: global.karlsend_address.clone(),
                prom_port: String::new(),
                print_stats: global.print_stats,
                log_to_file: instance.log_to_file.unwrap_or(global.log_to_file),
                health_check_port: String::new(),
                block_wait_time: instance.block_wait_time.unwrap_or(global.block_wait_time),
                min_share_diff: instance.min_share_diff,
                var_diff: instance.var_diff.unwrap_or(global.var_diff),
                shares_per_min: instance.shares_per_min.unwrap_or(global.shares_per_min),
                var_diff_stats: instance.var_diff_stats.unwrap_or(global.var_diff_stats),
                extranonce_size: instance.extranonce_size.unwrap_or(global.extranonce_size),
                pow2_clamp: instance.pow2_clamp.unwrap_or(global.pow2_clamp),
                coinbase_tag_suffix: global.coinbase_tag_suffix.clone(),
            };

            listen_and_serve_with_shutdown(
                bridge_config,
                Arc::clone(&karlsen_api_clone),
                if is_first_instance { Some(karlsen_api_clone) } else { None },
                instance_shutdown_rx,
            )
            .await
            .map_err(|e| format!("[Instance {}] Bridge server error: {}", instance_num, e))
        });
        instance_handles.push(handle);
    }

    tracing::info!("All {} instance(s) started, waiting for completion...", config.instances.len());

    let bridge_fut = async {
        let result = try_join_all(instance_handles).await;
        match result {
            Ok(_) => {
                tracing::info!("All instances completed successfully");
                Ok(())
            }
            Err(e) => {
                tracing::error!("One or more instances failed: {:?}", e);
                Err(anyhow::anyhow!("Instance error: {:?}", e))
            }
        }
    };

    tokio::pin!(bridge_fut);

    #[cfg(windows)]
    let mut shutdown_wait_rx = shutdown_rx.clone();
    let ctrl_c_fut = async move {
        #[cfg(windows)]
        {
            let _ = shutdown_wait_rx.wait_for(|v| *v).await;
        }
        #[cfg(not(windows))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    };

    tokio::pin!(ctrl_c_fut);

    tokio::select! {
        res = &mut bridge_fut => {
            res
        }
        _ = &mut ctrl_c_fut => {
            tracing::info!("Ctrl+C received, starting shutdown");

            #[cfg(not(windows))]
            {
                let _ = shutdown_tx.send(true);
                let res = tokio::select! {
                    res = &mut bridge_fut => res,
                    _ = tokio::signal::ctrl_c() => {
                        tracing::warn!("Second Ctrl+C received, forcing exit");
                        std::process::exit(130);
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                        tracing::warn!("Shutdown drain window elapsed, exiting");
                        Ok(())
                    }
                };


                if let Err(e) = res {
                    tracing::warn!("Shutdown completed with error: {e}");
                }
                return Ok(());
            }

            #[cfg(windows)]
            {
                let res = tokio::select! {
                    res = &mut bridge_fut => res,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                        tracing::warn!("Shutdown drain window elapsed, exiting");
                        Ok(())
                    }
                };


                if let Err(e) = res {
                    tracing::warn!("Shutdown completed with error: {e}");
                }
                return Ok(());
            }
        }
    }
}
