//! System figures for the HUD: global CPU, used memory and network throughput, once a second.

use std::time::{Duration, Instant};

use sysinfo::{Networks, System};
use tipsy_protocol::{Event, ToClient};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

/// How often a [`Event::Stats`] is produced.
pub const INTERVAL: Duration = Duration::from_secs(1);
/// The shortest interval between two network samples the rates are computed over. An interval
/// below this is not a measurement, and dividing by it would produce a meaningless spike.
const MIN_NET_INTERVAL: Duration = Duration::from_millis(1);

/// Starts the sampler; it stops on its own once the client end of `tx` is gone.
pub fn spawn(tx: UnboundedSender<ToClient>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut system = System::new();
        let mut networks = Networks::new_with_refreshed_list();
        // The first CPU reading has no interval to compare against, so it is a warm-up.
        system.refresh_cpu_usage();
        system.refresh_memory();
        // The network totals are counters since boot; only the difference between two samples is
        // throughput. The first tick has nothing to subtract from and reports no traffic.
        let mut previous = net_totals(&networks);
        let mut net_sampled_at = Instant::now();

        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            system.refresh_cpu_usage();
            system.refresh_memory();
            networks.refresh(true);

            let totals = net_totals(&networks);
            let (net_in, net_out) = net_rates(Some(previous), totals, net_sampled_at.elapsed());
            previous = totals;
            net_sampled_at = Instant::now();

            let stats = Event::Stats {
                cpu: system.global_cpu_usage(),
                ram: used_ram_percent(&system),
                net_in,
                net_out,
            };
            if tx.send(ToClient::Event(stats)).is_err() {
                // The client is gone: nothing left to report to.
                break;
            }
        }
    })
}

/// The byte totals every interface reports so far: `(received, transmitted)` since boot.
fn net_totals(networks: &Networks) -> (f64, f64) {
    (
        networks.list().values().map(|n| n.received() as f64).sum(),
        networks.list().values().map(|n| n.transmitted() as f64).sum(),
    )
}

/// Bytes per second in and out, from two cumulative `(received, transmitted)` samples and the time
/// between them. Without a previous sample — or over an interval too short to measure — there is no
/// rate to report and both directions are zero.
fn net_rates(previous: Option<(f64, f64)>, current: (f64, f64), elapsed: Duration) -> (f64, f64) {
    if previous.is_none() || elapsed < MIN_NET_INTERVAL {
        return (0.0, 0.0);
    }
    let previous = previous.expect("checked just above");
    let seconds = elapsed.as_secs_f64();
    (((current.0 - previous.0).max(0.0)) / seconds, ((current.1 - previous.1).max(0.0)) / seconds)
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

    #[test]
    fn no_previous_sample_reports_no_traffic() {
        // The totals are counters since boot; the first tick has nothing to subtract from.
        assert_eq!(net_rates(None, (10_000_000.0, 5_000_000.0), Duration::from_secs(1)), (0.0, 0.0));
    }

    #[test]
    fn rates_are_the_growth_between_two_samples() {
        let previous = (1_000_000.0, 500_000.0);
        let current = (1_002_048.0, 500_256.0);
        let (net_in, net_out) = net_rates(Some(previous), current, Duration::from_secs(1));
        assert!((net_in - 2048.0).abs() < 1e-6, "{net_in}");
        assert!((net_out - 256.0).abs() < 1e-6, "{net_out}");
    }

    #[test]
    fn rates_divide_by_the_real_interval() {
        let previous = (0.0, 0.0);
        let current = (4000.0, 1000.0);
        let (net_in, net_out) = net_rates(Some(previous), current, Duration::from_millis(250));
        assert!((net_in - 16000.0).abs() < 1e-6, "{net_in}");
        assert!((net_out - 4000.0).abs() < 1e-6, "{net_out}");
    }

    #[test]
    fn an_unmeasurable_interval_reports_no_traffic() {
        // A machine up for a day has terabytes of totals; dividing them by a zero interval would be
        // an infinity, and by a microsecond, an absurd spike.
        let big = (1_000_000_000_000.0, 1_000_000_000_000.0);
        assert_eq!(net_rates(Some(big), (big.0 + 1.0, big.1 + 1.0), Duration::ZERO), (0.0, 0.0));
        assert_eq!(net_rates(Some(big), big, Duration::from_micros(10)), (0.0, 0.0));
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
