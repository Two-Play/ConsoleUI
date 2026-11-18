//! Spike 11.1: the smallest web view ConsoleUI could ship. A fullscreen WebKitGTK 6 window without any
//! browser chrome; it logs timings and page events to stderr and takes commands on stdin, one per
//! line, the way the shell would steer it (later over D-Bus):
//!
//!   open <url> | back | forward | reload | stop | zoom <factor> | scroll <dy> | key <name> | quit
//!
//! Usage: webprobe [--data-dir DIR] <url>

use std::io::BufRead;
use std::time::Instant;

use webkit6::prelude::*;
use webkit6::{gio, glib, gtk};

fn main() -> glib::ExitCode {
    let start = Instant::now();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = match args.iter().position(|a| a == "--data-dir") {
        Some(i) if i + 1 < args.len() => {
            let dir = args.remove(i + 1);
            args.remove(i);
            Some(dir)
        }
        _ => None,
    };
    let url = args.first().cloned().unwrap_or_else(|| "https://example.org".into());

    let app = gtk::Application::builder()
        .application_id("org.consoleui.WebProbe")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| activate(app, &url, data_dir.as_deref(), start));
    // GTK must not parse our arguments
    app.run_with_args::<&str>(&[])
}

fn log(start: Instant, what: &str) {
    eprintln!("[{:7.3}s] {what}", start.elapsed().as_secs_f64());
}

fn activate(app: &gtk::Application, url: &str, data_dir: Option<&str>, start: Instant) {
    log(start, "gtk ready");
    // Data per profile (11.6): one network session per data directory
    let session = match data_dir {
        Some(dir) => webkit6::NetworkSession::new(Some(&format!("{dir}/data")), Some(&format!("{dir}/cache"))),
        None => webkit6::NetworkSession::new_ephemeral(),
    };
    let view = webkit6::WebView::builder().network_session(&session).build();
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&view) {
        settings.set_enable_developer_extras(false);
        settings.set_enable_smooth_scrolling(true);
        settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Always);
    }

    view.connect_load_changed(move |view, event| {
        let uri = view.uri().unwrap_or_default();
        log(start, &format!("load {event:?} {uri}"));
    });
    view.connect_title_notify(move |view| {
        log(start, &format!("title {:?}", view.title().unwrap_or_default()));
    });
    view.connect_estimated_load_progress_notify(move |view| {
        let p = view.estimated_load_progress();
        if p >= 1.0 || (p * 10.0).fract() < 0.05 {
            log(start, &format!("progress {:.0}%", p * 100.0));
        }
    });
    view.connect_load_failed_with_tls_errors(move |_, uri, _, errors| {
        log(start, &format!("tls error {errors:?} at {uri}"));
        true
    });
    // Popups (target=_blank, window.open) stay in this view instead of opening windows
    view.connect_create(move |view, action| {
        if let Some(uri) = action.request().and_then(|r| r.uri()) {
            log(start, &format!("popup -> same view: {uri}"));
            view.load_uri(&uri);
        }
        None
    });
    view.connect_permission_request(move |_, request| {
        log(start, &format!("permission request {}", request.type_().name()));
        request.deny();
        true
    });
    view.connect_web_process_terminated(move |_, reason| {
        log(start, &format!("web process terminated: {reason:?}"));
    });

    let window = gtk::ApplicationWindow::builder().application(app).title("ConsoleUI Web").child(&view).build();
    window.fullscreen();
    window.present();
    log(start, "window presented");
    view.load_uri(url);

    // Commands from stdin, read on a thread and handed to the main loop
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let app = app.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
        while let Ok(line) = rx.try_recv() {
            command(&app, &view, line.trim(), start);
        }
        glib::ControlFlow::Continue
    });
}

fn command(app: &gtk::Application, view: &webkit6::WebView, line: &str, start: Instant) {
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    log(start, &format!("command {line}"));
    match cmd {
        "open" => view.load_uri(arg),
        "back" => view.go_back(),
        "forward" => view.go_forward(),
        "reload" => view.reload(),
        "stop" => view.stop_loading(),
        "zoom" => view.set_zoom_level(arg.parse().unwrap_or(1.0)),
        "scroll" => js(view, &format!("window.scrollBy({{top: {}, behavior: 'smooth'}})", arg.parse::<f64>().unwrap_or(400.0))),
        "focusinput" => js(view, "document.querySelector('input,textarea')?.focus()"),
        "quit" => app.quit(),
        _ => log(start, "unknown command"),
    }
}

fn js(view: &webkit6::WebView, script: &str) {
    view.evaluate_javascript(script, None, None, gio::Cancellable::NONE, |_| {});
}
