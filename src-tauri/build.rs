fn main() {
    println!("cargo:rerun-if-env-changed=TUNEWIRE_REPORT_WEBHOOK");
    println!("cargo:rerun-if-env-changed=TUNEWIRE_SUPABASE_URL");
    println!("cargo:rerun-if-env-changed=TUNEWIRE_SUPABASE_ANON_KEY");
    tauri_build::build()
}
