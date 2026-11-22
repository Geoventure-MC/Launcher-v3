//! Essai de bout en bout, hors interface : installe Java, Minecraft et Forge dans un dossier,
//! synchronise le modpack du panel, puis (avec `--launch`) démarre le jeu avec un compte factice.
//!
//! ```text
//! cargo run --example install_e2e -- /tmp/geo-test            # installation seule
//! cargo run --example install_e2e -- /tmp/geo-test --launch   # + lancement (écran titre)
//! ```

use geoventure_launcher_lib::config::PanelClient;
use geoventure_launcher_lib::downloader::{cleanup_stale_mods, sync_files, SyncOptions};
use geoventure_launcher_lib::forge::{resolve_installed_forge_plan, ForgeResolveRequest};
use geoventure_launcher_lib::install::Installer;
use geoventure_launcher_lib::launch::spawn_resolved_forge;
use geoventure_launcher_lib::logs::LogStore;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let game_dir = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "/tmp/geo-test".into()));
    let launch = args.iter().any(|a| a == "--launch");
    let t0 = Instant::now();

    let panel = PanelClient::default_panel().unwrap();
    let cfg = panel
        .fetch_config_with_cache(None, "stable", &game_dir.join(".cache"))
        .await
        .expect("config du panel")
        .config;
    println!("Panel : MC {} / {}", cfg.game_version, cfg.loader.build);

    let installer = Installer {
        http: PanelClient::download_client().unwrap(),
        game_dir: game_dir.clone(),
        on_progress: Arc::new(|p| {
            if p.total == 0 || p.completed == p.total || p.completed % 500 == 0 {
                println!("[{}] {} {}/{} {}", p.phase, p.label, p.completed, p.total, p.current_file);
            }
        }),
        on_log: Arc::new(|lvl, m| println!("  {:?} {m}", lvl)),
    };

    let vanilla = installer.install_vanilla(&cfg.game_version).await.expect("vanilla");
    println!("== vanilla OK ({:?})", t0.elapsed());
    let java = installer.install_java(&vanilla).await.expect("java");
    println!("== java OK : {} ({:?})", java.display(), t0.elapsed());
    let id = installer
        .install_forge(&cfg.game_version, &cfg.loader.build, &java, &vanilla)
        .await
        .expect("forge");
    println!("== forge OK : {id} ({:?})", t0.elapsed());

    let manifest = panel.fetch_modpack_manifest(None, "stable").await.expect("manifeste");
    let report = sync_files(
        PanelClient::download_client().unwrap(),
        &manifest,
        &game_dir,
        &SyncOptions::modpack(cfg.ignored.clone()),
        |_| {},
    )
    .await
    .expect("modpack");
    let removed = cleanup_stale_mods(&game_dir, &manifest, &cfg.ignored).await.unwrap();
    println!(
        "== modpack : {} valides, {} téléchargés, {} erreurs, {} anciens mods retirés ({:?})",
        report.valid_count,
        report.downloaded_count,
        report.failed.len(),
        removed.len(),
        t0.elapsed()
    );

    let plan = resolve_installed_forge_plan(&ForgeResolveRequest {
        game_dir: game_dir.clone(),
        mc_version: cfg.game_version.clone(),
        loader_build: cfg.loader.build.clone(),
        username: "TestRust".into(),
        uuid: "00000000-0000-0000-0000-000000000001".into(),
        access_token: "0".into(),
        ram_min_gb: 2,
        ram_max_gb: 4,
        extra_jvm_args: vec![],
        server_host: None,
        server_port: None,
        width: Some(854),
        height: Some(480),
    })
    .expect("plan");
    println!(
        "== plan : {} jars, {} manquants {:?}",
        plan.classpath.len(),
        plan.missing_libraries.len(),
        plan.missing_libraries
    );
    assert!(plan.missing_libraries.is_empty(), "librairies manquantes");

    if launch {
        let logs = LogStore::default();
        let pid = spawn_resolved_forge(&java.to_string_lossy(), &game_dir, &plan, logs, |e| {
            println!("[mc] {}", e.message)
        })
        .await
        .expect("lancement");
        println!("== Minecraft démarré, PID {pid}");
        tokio::time::sleep(std::time::Duration::from_secs(
            std::env::var("E2E_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(240),
        ))
        .await;
    }
}
