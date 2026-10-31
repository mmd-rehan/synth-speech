//! The runtime watchdog: given live memory/swap/thermal signals, decides what
//! protective action to take. Pure so it can be tested with synthetic traces.

use oas_hardware::ThermalState;

/// Watchdog thresholds.
#[derive(Debug, Clone, Copy)]
pub struct WatchdogConfig {
    /// The governor's safety reserve; action triggers below half of it.
    pub safety_reserve: u64,
    /// Swap growth over the window that signals a swap storm.
    pub swap_growth_limit: u64,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        WatchdogConfig {
            safety_reserve: 3 * 1024 * 1024 * 1024 / 2, // 1.5 GB
            swap_growth_limit: 500 * 1024 * 1024,       // 500 MB / 10 s
        }
    }
}

/// A protective action the scheduler should take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    /// Pause the newest running job and warn.
    PauseNewest(&'static str),
    /// Reduce running jobs to one (thermal Serious).
    ReduceToOne(&'static str),
    /// Pause everything (thermal Critical).
    PauseAll(&'static str),
}

/// Decide what to do given the current signals and recent swap growth.
pub fn decide(
    available_ram: u64,
    swap_growth_over_window: u64,
    thermal: ThermalState,
    config: &WatchdogConfig,
) -> Vec<WatchdogAction> {
    let mut actions = Vec::new();

    if available_ram < config.safety_reserve / 2 {
        actions.push(WatchdogAction::PauseNewest(
            "Your computer is very low on memory",
        ));
    }
    if swap_growth_over_window > config.swap_growth_limit {
        actions.push(WatchdogAction::PauseNewest(
            "Your computer is running out of memory",
        ));
    }
    match thermal {
        ThermalState::Serious => actions.push(WatchdogAction::ReduceToOne(
            "The machine is hot; reducing to one job",
        )),
        ThermalState::Critical => {
            actions.push(WatchdogAction::PauseAll("The machine is too hot; pausing"))
        }
        _ => {}
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn calm_conditions_take_no_action() {
        let actions = decide(8 * GB, 0, ThermalState::Nominal, &WatchdogConfig::default());
        assert!(actions.is_empty());
    }

    #[test]
    fn low_memory_pauses_newest() {
        let actions = decide(
            200 * 1024 * 1024, // 200 MB, below 0.75 GB half-reserve
            0,
            ThermalState::Nominal,
            &WatchdogConfig::default(),
        );
        assert!(actions
            .iter()
            .any(|a| matches!(a, WatchdogAction::PauseNewest(_))));
    }

    #[test]
    fn swap_storm_pauses_newest() {
        let actions = decide(
            8 * GB,
            700 * 1024 * 1024, // 700 MB swap growth
            ThermalState::Nominal,
            &WatchdogConfig::default(),
        );
        assert!(actions
            .iter()
            .any(|a| matches!(a, WatchdogAction::PauseNewest(_))));
    }

    #[test]
    fn thermal_serious_reduces_and_critical_pauses_all() {
        let serious = decide(8 * GB, 0, ThermalState::Serious, &WatchdogConfig::default());
        assert!(serious.contains(&WatchdogAction::ReduceToOne(
            "The machine is hot; reducing to one job"
        )));
        let critical = decide(
            8 * GB,
            0,
            ThermalState::Critical,
            &WatchdogConfig::default(),
        );
        assert!(critical.contains(&WatchdogAction::PauseAll("The machine is too hot; pausing")));
    }
}
