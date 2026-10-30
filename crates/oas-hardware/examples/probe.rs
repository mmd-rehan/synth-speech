//! Print this machine's profile and a live usage sample. Handy for manual
//! verification: `cargo run -p oas-hardware --example probe`.

use std::time::Instant;

use oas_hardware::{build_profile, RealProbe, SystemProbe};

fn main() {
    let mut probe = RealProbe::new();
    let models_dir = std::env::temp_dir();

    let start = Instant::now();
    let profile = build_profile(&probe, &models_dir);
    let elapsed = start.elapsed();

    println!("profile built in {} ms", elapsed.as_millis());
    match serde_json::to_string_pretty(&profile) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("serialise failed: {e}"),
    }

    let sample = probe.sample_usage(&models_dir);
    println!(
        "\nusage sample: cpu {:.1}% over {} cores, ram used {} MiB",
        sample.cpu_total_pct,
        sample.cpu_per_core.len(),
        sample.ram_used_bytes / (1024 * 1024),
    );
}
