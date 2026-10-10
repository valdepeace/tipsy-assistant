//! System figures for the HUD: global CPU, used memory and network throughput, once a second.

use std::time::Duration;

use sysinfo::{Networks, System};
use tipsy_protocol::{Event, ToClient};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

/// How often a [`Event::Stats`] is produced.
pub const INTERVAL: Duration = Duration::from_secs(1);

/// Starts the sampler; it stops on its own once the client end of `tx` is gone.
pub fn spawn(tx: UnboundedSender<ToClient>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut system = System::new();
        let mut networks = Networks::new_with_refreshed_list();
        // The first CPU reading has no interval to compare against, so it is a warm-up.
        system.refresh_cpu_usage();
        system.refresh_memory();

        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            system.refresh_cpu_usage();
            system.refresh_memory();
            networks.refresh(true);

            let stats = Event::Stats {
                cpu: system.global_cpu_usage(),
                ram: used_ram_percent(&system),
                net_in: networks.list().values().map(|n| n.received() as f64).sum(),
                net_out: networks.list().values().map(|n| n.transmitted() as f64).sum(),
            };
            if tx.send(ToClient::Event(stats)).is_err() {
                // The client is gone: nothing left to report to.
                break;
            }
        }
    })
}

/// Used memory as a percentage of the total, or 0 when the total is not known yet.
fn used_ram_percent(system: &System) -> f32 {
    let total = system.total_memory();
    if total == 0 {
        return 0.0;
    }
    (system.used_memory() as f64 * 100.0 / total as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_percentage_is_bounded() {
        let system = System::new();
        let percent = used_ram_percent(&system);
        assert!((0.0..=100.0).contains(&percent), "{percent}");
    }

    #[tokio::test]
    async fn reports_stats_and_stops_when_the_client_goes_away() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = spawn(tx);
        let first = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
        assert!(matches!(first, Ok(Some(ToClient::Event(Event::Stats { .. })))), "{first:?}");

        drop(rx);
        // The sampler notices the closed channel at the next tick and finishes.
        tokio::time::timeout(Duration::from_secs(3), handle).await.expect("sysmon must stop").expect("no panic");
    }
}
