use axum_server::tls_rustls::RustlsConfig;
use instance::IOperation;
use log::{debug, error, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sysinfo::get_system_info;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use std::{net::SocketAddr, process::exit};
use tokio::signal;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;

mod sysinfo;
mod instance;
mod server;

#[derive(Serialize, Deserialize)]
struct AppConfig {
    listen: String,
    token: String,
    cert: String,
    key: String,
}

static APP_CONFIG: Lazy<AppConfig> = Lazy::new(|| {
    //Load config file
    let c: AppConfig =
        serde_json::from_str(&std::fs::read_to_string("config.json").unwrap()).unwrap();
    c
});

static EXPECTED_TOKEN: Lazy<String> = Lazy::new(|| {
    let s = "Bearer ".to_owned() + &APP_CONFIG.token;
    s
});

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_BACKTRACE", "1");
    std::env::set_var("RUST_LOG", "debug");
    env_logger::init();

    let (iop_sender, iop_receiver) = mpsc::channel::<instance::IOperation>(32);

    debug!("{}", json!(get_system_info()));
    tokio::task::spawn(sysinfo::sys_info_getter());

    tokio::task::spawn(instance::instance_broker(iop_receiver, iop_sender.clone()));

    let iop_sender_server = iop_sender.clone();

    if (APP_CONFIG.cert == "") || (APP_CONFIG.key == "") {
        //Disable SSL
        info!("Listening on http://{}", &APP_CONFIG.listen);
        tokio::task::spawn(async move {
            let listener = tokio::net::TcpListener::bind(&APP_CONFIG.listen)
                .await
                .unwrap();
            axum::serve(
                listener,
                server::get_router(iop_sender_server)
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap()
        });
    } else {
        //Enable SSL
        info!("Listening on https://{}", &APP_CONFIG.listen);

        let config = RustlsConfig::from_pem_file(
            PathBuf::from(&APP_CONFIG.cert),
            PathBuf::from(&APP_CONFIG.key),
        )
        .await
        .expect("Loading pem file failed");

        let addr = SocketAddr::from_str(&APP_CONFIG.listen).unwrap();

        tokio::task::spawn(async move {
            axum_server::bind_rustls(addr, config)
                .serve(
                    server::get_router(iop_sender_server)
                        .into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap()
        });
    }

    debug!("Starting auto restart task");
    // Keep alive
    let iop_sender_keepalive = iop_sender.clone();
    tokio::task::spawn(async move {

        loop {
            sleep(Duration::from_secs(10)).await;

            let op = IOperation{
                otype: instance::IOperationTypes::KeepInstanceRunning,
                result: None
            };
            let _ = iop_sender_keepalive.send(op).await;
        }
    });

    debug!("Waiting for stop signal");
    // Signal handling
    let signal = signal::ctrl_c().await;
    match signal {
        Ok(()) => {
            info!("Received shutdown signal, exiting...");
            let (sender, receiver) = oneshot::channel();
            let _ = iop_sender
                .send(instance::IOperation {
                    otype: instance::IOperationTypes::SaveConfiguration,
                    result: Some(sender),
                })
                .await;
            let result = receiver.await;
            match result {
                Ok(result) => match result {
                    Ok(_) => {
                        info!("Saved instance configuration");
                    }
                    Err(result) => {
                        error!("Saving the instance configuration failed: {}", result);
                    }
                },
                Err(result) => {
                    error!(
                        "Getting the result of saving the instance configuration failed: {}",
                        result
                    );
                }
            }

            let (sender, receiver) = oneshot::channel();
            let _ = iop_sender
                .send(instance::IOperation {
                    otype: instance::IOperationTypes::Shutdown,
                    result: Some(sender),
                })
                .await;
            let _ = receiver.await;
            info!("Bye!");
            exit(0);
        }
        Err(err) => {
            error!("Error receiving shutdown signal: {}", err);
            exit(-1);
        }
    }
}
