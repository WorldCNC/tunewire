// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    linux_gpu_workarounds();
    tunewire_lib::run()
}

/// WebKitGTK on NVIDIA's driver (Wayland especially) flickers: frames handed
/// to the compositor as GPU buffers are sometimes shown before they're fully
/// drawn, so hovers, menus and dialogs blink. Turning off WebKit's DMA-BUF
/// renderer and NVIDIA's explicit sync avoids it. Only on NVIDIA, only when
/// the user hasn't set these themselves; TUNEWIRE_GPU=default skips it.
#[cfg(target_os = "linux")]
fn linux_gpu_workarounds() {
    if std::env::var("TUNEWIRE_GPU").as_deref() == Ok("default") {
        return;
    }
    let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists() || std::path::Path::new("/sys/module/nvidia").exists();
    if !nvidia {
        return;
    }
    // Set before any thread exists (WebKit reads them at startup).
    for (key, value) in [("WEBKIT_DISABLE_DMABUF_RENDERER", "1"), ("__NV_DISABLE_EXPLICIT_SYNC", "1")] {
        if std::env::var_os(key).is_none() {
            // SAFETY: called first thing in main, single-threaded.
            unsafe { std::env::set_var(key, value) };
        }
    }
}
