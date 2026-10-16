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
mod commands;
mod controller;
mod decoder;
mod discord;
mod downloads;
mod error;
mod http;
mod localfiles;
mod lyrics;
mod models;
mod plugins;
mod search;
mod secrets;
mod spotdl;
mod spotify;
mod state;
mod store;
mod stream;
mod themes;
mod ytdlp;

#[cfg(test)]
mod itests;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

use audio::AudioHandle;
use commands::*;
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

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            app.manage(AppState {
                store,
                themes,
                plugins,
                cache: Arc::new(SearchCache::default()),
                queue: Mutex::new(QueueState::default()),
                now: Mutex::new(NowPlaying { phase: Phase::Idle, item: None, version: None, message: None }),
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
                last_full: Mutex::new(String::new()),
                search_gen: AtomicU64::new(0),
                volume_save_gen: AtomicU64::new(0),
            });
            system::apply_settings(&app.state::<AppState>(), &settings);

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
