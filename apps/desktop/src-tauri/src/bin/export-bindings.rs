//! Developer tool: regenerate `apps/desktop/src/bindings.ts` from the Rust
//! command definitions. Run by `scripts/check.sh` and CI, then the result is
//! diff-checked so the committed bindings can never drift.

fn main() {
    match oas_desktop_lib::export_bindings() {
        Ok(()) => println!("Exported TypeScript bindings."),
        Err(e) => {
            eprintln!("failed to export bindings: {e}");
            std::process::exit(1);
        }
    }
}
