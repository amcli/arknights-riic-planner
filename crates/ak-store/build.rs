// `sqlx::migrate!` embeds the files in `migrations/` but only tracks the ones
// that already exist; this rebuilds the crate when one is added.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
