mod args;
mod config;
mod proxy;
mod tls;

use crate::args::Args;
use crate::config::Config;
use actix_web::{web, App, HttpServer, Route};
use std::process::exit;
use tracing::error;
use tracing_subscriber::layer::SubscriberExt;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    configure_tracing();

    let config = match Config::new(&args.config).await {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to read configuration: {e}");
            exit(1);
        }
    };

    // Configure TLS, if needed
    let tls_config = match &config.tls {
        Some(config) => match tls::configure_tls(&config.pubkey, &config.privkey).await {
            Ok(x) => Some(x),
            Err(e) => {
                error!("Failed to configure TLS: {e}");
                exit(1);
            }
        },
        None => None,
    };

    let appdata = web::Data::new(config.clone());
    let http_server = HttpServer::new(move || {
        App::new()
            .wrap(tracing_actix_web::TracingLogger::default())
            .app_data(appdata.clone())
            .default_service(Route::new().to(proxy::proxy))
    });

    // Bind the server to the provided bind address and port
    // Configure TLS as the user specifies
    let bind_url = format!("{}:{}", config.net.bind_address, config.net.port);
    let http_server = if let Some(tls_config) = tls_config {
        http_server.bind_rustls(bind_url, tls_config)
    } else {
        http_server.bind(bind_url)
    };

    let http_server = match http_server {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to bind the HTTP server: {e}");
            exit(1);
        }
    };

    http_server.run().await
}

/// Configure the tracing logger according to the provided log level
fn configure_tracing() {

    let tracing_sub = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact())
        .with(tracing_subscriber::EnvFilter::from_default_env());

    tracing::subscriber::set_global_default(tracing_sub).expect("configuring tracing");
}
