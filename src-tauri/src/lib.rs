//! Tunewire — native multi-source music player.
//!
//! Module map:
//!   state       shared AppState, playback snapshot, event helpers
//!   controller  resolve → stream → play, fallback, gapless preloading
//!   commands/*  IPC commands grouped by area
//!   ytdlp       yt-dlp runner (semaphore, retry/backoff, errors, bundled copy)
//!   spotdl      spotDL runner (Spotify search / playlists)
//!   search      normalization, fuzzy dedup, cross-platform grouping, cache
//!   spotify     Spotify embed-page metadata
//!   artist      artist / album pages
//!   lyrics      LRCLIB lyrics
//!   stream      disk-backed HTTP / HLS / local reader
//!   decoder     symphonia decoder (+ Opus via libopus)
//!   audio       rodio engine: gapless, crossfade, normalization
//!   downloads   offline downloads;  localfiles  local music library
//!   store       settings + library persistence;  secrets  OS keychain
//!   themes, plugins, discord

mod artist;
mod audio;
mod backup;
mod cloud;
mod commands;
mod controller;
mod css_guard;
mod decoder;
mod discord;
mod downloads;
mod eq;
mod error;
mod explore;
mod foryou;
mod gallery;
mod http;
mod imports;
mod localfiles;
mod loopback;
mod lyrics;
mod matcher;
mod media;
mod models;
mod plugins;
mod radio;
mod report;
mod scrobble;
mod search;
mod secrets;
mod soundcloud;
mod spotdl;
mod spotify;
mod spotify_lib;
mod spotify_play;
mod state;
mod stats;
mod store;
mod stream;
mod themes;
mod ytdlp;
mod ytdlp_worker;
mod ytm;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod itests;
#[cfg(test)]
mod perfbench;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

use audio::AudioHandle;
use commands::*;

// Returns freed memory to the OS more readily than the system allocator with
// our many short-lived threads (lower resident memory over long sessions).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use plugins::PluginStore;
use search::SearchCache;
use state::{AppState, NowPlaying, Phase, QueueState};
use store::Store;
use themes::ThemeStore;

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Play / Pause", true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", "Next", true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", "Previous", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &prev, &next, &sep, &show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Tunewire")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| {
            let st = app.state::<AppState>();
            match ev.id.as_ref() {
                "toggle" => playback::toggle_pause(app.clone(), st),
                "next" => playback::next_track(app.clone()),
                "prev" => playback::previous_track(app.clone(), st),
                "show" => show_main(app),
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub(crate) fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    report::install_panic_hook();
    tauri::Builder::default()
        // One app window: opening a tunewire:// link (or the app again) goes
        // to the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            show_main(app);
            if let Some(url) = argv.iter().find(|a| a.starts_with("tunewire://")) {
                let _ = tauri::Emitter::emit(app, "deep-link", url.clone());
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let cache = app.path().app_cache_dir().unwrap_or_else(|_| data.join("cache"));
            stream::set_cache_dir(cache.join("streams"));
            let store = Store::open(data.clone())?;
            localfiles::init(&data);
            spotify_play::init(data.clone());

            // One-time migration: move a plain-text Spotify secret into the keychain.
            let legacy = store.settings().spotify_client_secret;
            if !legacy.is_empty() && secrets::set(secrets::SPOTIFY_SECRET, &legacy).is_ok() {
                let _ = store.update_settings(|s| s.spotify_client_secret.clear());
            }

            let themes = ThemeStore::new(data.join("themes"));
            let plugins = PluginStore::new(data.join("plugins"));
            if !store.settings().example_plugin_installed && plugins.install_example().is_ok() {
                let _ = store.update_settings(|s| s.example_plugin_installed = true);
            }
            let discord = discord::DiscordHandle::spawn();
            let handle = app.handle().clone();
            let audio = AudioHandle::spawn(store.settings().volume, move |ev| controller::on_engine_event(&handle, ev));
            let settings = store.settings();
            // Last session's queue, restored paused (play resumes where it was).
            let saved = state::SavedQueue::load(&data);
            let current_item = saved.current.and_then(|i| saved.items.get(i).cloned());
            let resume = current_item.as_ref().filter(|_| saved.position > 3.0).map(|i| (i.uid, saved.position));
            let queue =
                QueueState { next_uid: saved.items.iter().map(|i| i.uid).max().unwrap_or(0), current: current_item.as_ref().and(saved.current), items: saved.items, resume_at: None, unshuffled: None };
            app.manage(AppState {
                store,
                themes,
                plugins,
                cache: Arc::new(SearchCache::default()),
                queue: Mutex::new(queue),
                now: Mutex::new(NowPlaying { phase: Phase::Idle, item: current_item, version: None, message: None }),
                audio,
                load_gen: AtomicU64::new(0),
                next_token: AtomicU64::new(0),
                current_token: AtomicU64::new(0),
                preload: Mutex::new(None),
                preload_gen: AtomicU64::new(0),
                preload_inflight: AtomicBool::new(false),
                discord,
                consecutive_failures: AtomicU32::new(0),
                data_dir: data,
                last_full: AtomicU64::new(0),
                search_gen: AtomicU64::new(0),
                volume_save_gen: AtomicU64::new(0),
                resume_position: Mutex::new(resume),
                queue_save_gen: AtomicU64::new(0),
                queue_saved_at: Mutex::new(std::time::Instant::now()),
            });
            system::apply_settings(&app.state::<AppState>(), &settings);
            {
                let st = app.state::<AppState>();
                let history = st.store.library.lock().unwrap().history.clone();
                stats::init(&st.store.dir, &history);
                scrobble::init(&st.store.dir);
                matcher::init(&st.store.dir);
                soundcloud::init(&st.store.dir);
                let s = st.store.settings();
                cloud::init(&st.store.dir, &s.supabase_url, &s.supabase_anon_key);
            }
            // Finish matching Spotify songs left over from last time.
            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                matcher::kick(&h);
            });
            // Build today's mixes in the background so Home opens instantly.
            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                let st = h.state::<AppState>();
                if commands::foryou::for_you(st.clone()).stale && commands::foryou::refresh_for_you(st, None).await.is_ok() {
                    let _ = tauri::Emitter::emit(&h, "for-you-ready", ());
                }
            });
            // Python + yt_dlp import take ~0.5 s; pay it now, not on the first song.
            let bin_dir = (settings.ytdlp_source == "bundled").then(|| app.state::<AppState>().data_dir.join("bin"));
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                // A bundled standalone yt-dlp from an older version: switch to
                // the zipapp when Python can run it (the worker needs it).
                if let Some(dir) = bin_dir {
                    if tauri::async_runtime::spawn_blocking(move || ytdlp::upgrade_bundled(&dir)).await.unwrap_or(false) {
                        ytdlp_worker::reset().await;
                    }
                }
                ytdlp_worker::warm(ytdlp::binary()).await;
                ytdlp::warm_youtube().await;
            });
            // Tokens/ids the first search would otherwise wait for.
            tauri::async_runtime::spawn_blocking(|| {
                soundcloud::warm();
                spotify::warm();
            });
            // Open the connections a Spotify song's first match uses.
            tauri::async_runtime::spawn(async {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let _ = spotify::isrc("4uLU6hMCjMI75M1A2tKUQC").await;
                let _ = ytm::search_songs("GBARL9300135").await;
            });

            // Pick up new/removed files in the local library.
            if !settings.local_folders.is_empty() {
                let folders = settings.local_folders.clone();
                let h = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if localfiles::scan(&folders).is_ok() {
                        state::emit_library(&h, "local");
                    }
                });
            }
            // tunewire:// links: register (Linux/AppImage needs it at runtime)
            // and forward links that arrive while running.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
                let _ = app.deep_link().register_all();
                let h = app.handle().clone();
                app.deep_link().on_open_url(move |ev| {
                    for u in ev.urls() {
                        let _ = tauri::Emitter::emit(&h, "deep-link", u.to_string());
                    }
                });
            }
            // Logout/shutdown (SIGTERM) and Ctrl+C: exit normally, so the Exit
            // handler below saves the library, stats and queue.
            #[cfg(unix)]
            {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
                    std::future::poll_fn(|cx| if term.poll_recv(cx).is_ready() || int.poll_recv(cx).is_ready() { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }).await;
                    h.exit(0);
                });
            }
            media::init(app);
            media::update(app.handle(), &app.state::<AppState>().playback_state());
            if let Err(e) = setup_tray(app) {
                // Some Linux desktops have no tray; that's fine.
                eprintln!("tunewire: tray unavailable: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let to_tray = app.try_state::<AppState>().map(|s| s.store.settings().close_to_tray).unwrap_or(false);
                if to_tray && app.tray_by_id("main").is_some() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            system::health_check,
            system::tools_status,
            system::discord_status,
            system::get_settings,
            system::update_settings,
            system::set_spotify_secret,
            system::spotify_login,
            system::spotify_logout,
            system::spotify_account,
            system::list_output_devices,
            system::report_error,
            system::reports_available,
            system::ytdlp_install,
            discover::search_tracks,
            discover::load_playlist,
            discover::extract_stream_url,
            discover::play_url,
            discover::get_lyrics,
            discover::artist_page,
            discover::artist_albums,
            discover::album_page,
            discover::album_cover,
            discover::complete_metadata,
            playback::play_track,
            playback::play_tracks,
            playback::start_radio,
            playback::prefetch_track,
            commands::browse::ytm_search_extras,
            commands::browse::ytm_artist,
            commands::browse::ytm_collection,
            commands::browse::ytm_moods,
            commands::browse::ytm_find_album,
            commands::browse::ytm_mood_page,
            commands::browse::artist_soundcloud,
            commands::browse::spotify_library_login,
            commands::browse::spotify_library_import,
            commands::browse::spotify_profile_playlists,
            commands::browse::import_apple_music,
            commands::browse::import_rows,
            discover::apply_matches,
            library::remove_many_from_playlist,
            library::move_many_in_playlist,
            library::update_playlist_meta,
            library::set_playlist_cloud,
            library::replace_playlist_tracks,
            library::replace_favorites,
            library::load_sync_state,
            library::save_sync_state,
            playback::set_sleep_timer,
            commands::queue::set_shuffle,
            commands::cloud::cloud_config,
            commands::cloud::cloud_session,
            commands::cloud::cloud_sign_in,
            commands::cloud::cloud_sign_out,
            commands::cloud::cloud_prepare_restart,
            commands::cloud::cancel_sign_in,
            commands::extras::backup_file_name,
            commands::extras::export_backup,
            commands::extras::inspect_backup,
            commands::extras::restore_backup,
            commands::extras::list_snapshots,
            commands::extras::restore_playlist,
            commands::extras::restore_history,
            commands::extras::scrobble_status,
            commands::extras::set_lastfm_app,
            commands::extras::lastfm_connect,
            commands::extras::listenbrainz_connect,
            commands::extras::scrobble_disconnect,
            commands::extras::gallery_index,
            commands::extras::gallery_install,
            commands::foryou::for_you,
            commands::foryou::refresh_for_you,
            commands::foryou::listening_stats,
            commands::foryou::save_share_image,
            commands::explore::search_suggestions,
            commands::explore::search_preview,
            commands::explore::search_history,
            commands::explore::clear_search_history,
            commands::explore::chart_countries,
            commands::explore::get_charts,
            commands::explore::list_followed,
            commands::explore::follow_artist,
            commands::explore::new_releases,
            commands::explore::mark_releases_seen,
            commands::explore::translate_lyrics,
            commands::explore::cover_color,
            commands::explore::cover_wash,
            commands::explore::spotify_user_playlists,
            commands::explore::sync_playlist,
            commands::explore::background_image_data,
            playback::play_index_cmd,
            playback::next_track,
            playback::previous_track,
            playback::pause,
            playback::resume,
            playback::toggle_pause,
            playback::stop,
            playback::set_volume,
            playback::seek,
            playback::get_playback_state,
            playback::set_repeat,
            queue::get_queue,
            queue::add_to_queue,
            queue::remove_from_queue,
            queue::clear_queue,
            queue::reorder_queue,
            queue::shuffle_queue,
            queue::set_item_platform,
            library::list_playlists,
            library::create_playlist,
            library::rename_playlist,
            library::delete_playlist,
            library::add_to_playlist,
            library::remove_from_playlist,
            library::move_in_playlist,
            library::recommend_for_playlist,
            library::list_favorites,
            library::toggle_favorite,
            library::list_history,
            library::clear_history,
            offline::download_tracks,
            offline::list_downloads,
            offline::delete_download,
            offline::downloads_dir,
            offline::add_local_folder,
            offline::remove_local_folder,
            offline::rescan_local,
            offline::list_local,
            customize::list_themes,
            customize::load_theme,
            customize::get_theme,
            customize::save_theme,
            customize::delete_theme,
            customize::export_theme,
            customize::import_theme,
            customize::list_plugins,
            customize::enable_plugin,
            customize::disable_plugin,
            customize::get_plugin_source,
            customize::set_plugin_setting,
            customize::plugins_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, ev| {
            if let tauri::RunEvent::Exit = ev {
                // A sign-in refresh in flight saves its new token first.
                cloud::freeze_for_restart(true);
                stats::flush();
                // Library changes still waiting for the background writer, and
                // where the song was.
                if let Some(st) = app.try_state::<AppState>() {
                    st.save_all_now();
                }
            }
        });
}
