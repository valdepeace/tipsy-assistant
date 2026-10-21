use tauri::{LogicalSize, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use url::Url;

const FULL: LogicalSize<f64> = LogicalSize::new(1120.0, 760.0);
const COMPACT: LogicalSize<f64> = LogicalSize::new(220.0, 165.0);
const DEFAULT_URL: &str = "http://127.0.0.1:7070/";

fn local_url() -> Result<Url, String> {
    let raw = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_URL.to_owned());
    validate_url(&raw)
}

fn validate_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("invalid URL: {error}"))?;
    let local = match url.host().ok_or("URL has no host")? {
        url::Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
    };
    if url.scheme() != "http" || !local {
        return Err("the spike only loads an HTTP URL on localhost".to_owned());
    }
    Ok(url)
}

fn main() {
    let mut url = match local_url() {
        Ok(url) => url,
        Err(error) => {
            eprintln!("tipsy-floating-window-spike: {error}");
            std::process::exit(2);
        }
    };
    url.query_pairs_mut().append_pair("floating", "1");

    tauri::Builder::default()
        .setup(move |app| {
            let window = WebviewWindowBuilder::new(app, "tipsy", WebviewUrl::External(url.clone()))
                .title("Tipsy")
                .inner_size(FULL.width, FULL.height)
                .always_on_top(true)
                .center()
                .build()?;
            let on_focus_window = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(focused) = event {
                    if *focused {
                        move_to_bottom_right(&on_focus_window, FULL);
                        let _ = on_focus_window.set_size(FULL);
                    } else {
                        let _ = on_focus_window.set_size(COMPACT);
                        move_to_bottom_right(&on_focus_window, COMPACT);
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run floating window spike");
}

fn move_to_bottom_right(window: &tauri::WebviewWindow, size: LogicalSize<f64>) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let monitor_size = monitor.size();
    let margin = (24.0 * scale) as i32;
    let width = (size.width as f64 * scale) as u32;
    let height = (size.height as f64 * scale) as u32;
    let x = monitor.position().x + monitor_size.width as i32 - width as i32 - margin;
    let y = monitor.position().y + monitor_size.height as i32 - height as i32 - margin;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_must_be_http_and_loopback() {
        assert!(validate_url("http://localhost:7070/").is_ok());
        assert!(validate_url("http://127.0.0.1:7070/").is_ok());
        assert!(validate_url("http://[::1]:7070/").is_ok());
        assert!(validate_url("https://localhost:7070/").is_err());
        assert!(validate_url("http://example.com/").is_err());
    }

    #[test]
    fn compact_window_is_smaller_than_full_window() {
        assert!(COMPACT.width < FULL.width && COMPACT.height < FULL.height);
        assert_eq!((COMPACT.width, COMPACT.height), (220.0, 165.0));
    }
}
