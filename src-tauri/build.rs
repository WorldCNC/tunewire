fn main() {
    println!("cargo:rerun-if-env-changed=TUNEWIRE_REPORT_WEBHOOK");
    tauri_build::build()
}
