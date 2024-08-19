use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Json, Path, Request};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use log::{error, info};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tower_http::cors::CorsLayer;

use crate::instance::{self, IOperationTypes};
use crate::EXPECTED_TOKEN;

#[derive(Serialize, Deserialize, Clone)]
pub struct SrvMessage {
    message: String,
}

pub fn get_router(operation_channel: mpsc::Sender<instance::IOperation>) -> Router {
    let routes_api = Router::new()
        .route("/instances/create", put(r_instance_create))
        .route("/instances/start/:name", post(r_instance_start))
        .route("/instances/stop/:name", post(r_instance_stop))
        .route("/instances/halt/:name", post(r_instance_halt))
        .route("/instances/:name", delete(r_instance_remove))
        .route("/instances/execute/:name", post(r_instance_execute))
        .route("/instances", get(r_instance_list))
        .route("/instances/save", post(r_instance_save))
        .route("/instances/:name", post(r_instance_update))
        .layer(middleware::from_fn(m_authentication))
        .route("/instances/log_reader/:name", get(ws_log_reader));
    Router::new()
        .nest("/api", routes_api)
        .layer(Extension(operation_channel))
        .layer(CorsLayer::permissive())
        .fallback(r_undefined)
        .layer(middleware::from_fn(m_logging))
}

async fn m_authentication(request: Request, next: Next) -> Response {
    match request.headers().get("authorization") {
        None => return (StatusCode::UNAUTHORIZED).into_response(),
        Some(a) => {
            if a != EXPECTED_TOKEN.as_str() {
                return (StatusCode::UNAUTHORIZED).into_response();
            }
        }
    }

    let response = next.run(request).await;
    response
}

async fn m_logging(request: Request, next: Next) -> Response {
    // Extract the client's IP address from the connection info
    let ConnectInfo(addr): ConnectInfo<SocketAddr> = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .cloned()
        .unwrap();
    let ip = addr.ip();

    // Get the URI path and request method
    let uri_path = request.uri().path().to_string();
    let method = request.method().to_string();

    // Process the request
    let response = next.run(request).await;

    // Get the response status code
    let status = response.status().as_u16();

    // Log the information
    info!(
        "IP: {}, URI Path: {}, Method: {}, Status: {}",
        ip, uri_path, method, status
    );

    response
}

async fn r_undefined() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Body::empty())
}

async fn r_instance_create(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Json(instance_info): Json<instance::InstanceInfo>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Create(instance_info),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(result) => match result {
                instance::OkResult::None => StatusCode::OK.into_response(),
                _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            },
            Err(msg) => {
                return (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg }))
                    .into_response();
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_start(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Start(name),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_stop(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Stop(name),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_halt(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Halt(name),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_remove(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Remove(name),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_execute(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Path(name): Path<String>,
    command: String,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Execute(name, command),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_save(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::SaveConfiguration,
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_list(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
) -> impl IntoResponse {
    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::ListInstances,
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(instance::OkResult::Message(msg)) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                msg,
            )
                .into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn r_instance_update(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    Path(name): Path<String>,
    Json(new_info): Json<instance::InstanceInfo>,
) -> impl IntoResponse {
    if name != new_info.name {
        return (
            StatusCode::FORBIDDEN,
            Json(SrvMessage {
                message: "Changing instance name is not allowed".to_string(),
            }),
        )
            .into_response();
    }

    let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
    let operation = instance::IOperation {
        otype: IOperationTypes::Update(name, new_info),
        result: Some(r_sender),
    };

    let _ = operation_sender.send(operation).await;

    let result = r_receiver.await;
    match result {
        Ok(result) => match result {
            Ok(_) => StatusCode::OK.into_response(),
            Err(msg) => {
                (StatusCode::BAD_GATEWAY, Json(SrvMessage { message: msg })).into_response()
            }
        },
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SrvMessage {
                message: "Failed to receive response".to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ws_log_reader(
    Extension(operation_sender): Extension<mpsc::Sender<instance::IOperation>>,
    ws: WebSocketUpgrade,
    //ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, operation_sender, name))
}

async fn handle_socket(
    mut socket: WebSocket,
    operation_sender: mpsc::Sender<instance::IOperation>,
    name: String,
) {
    //Auth
    if let Some(msg) = socket.recv().await {
        match msg {
            Ok(msg) => match msg.into_text() {
                Ok(auth) if auth == *EXPECTED_TOKEN => {},
                _ => {
                    let _ = socket.close().await;
                    return;
                }
            },
            Err(_) => {
                let _ = socket.close().await;
                return;
            }
        }
    } else {
        let _ = socket.close().await;
        return;
    };
    info!("A websocket client connected to the server.");
    //Get log once
    {
        let (r_sender, r_receiver) = oneshot::channel::<instance::InstanceOperationResult>();
        let operation = instance::IOperation {
            otype: IOperationTypes::StdoutRead(name.clone()),
            result: Some(r_sender),
        };
        let _ = operation_sender.send(operation).await;
        let result = r_receiver.await;
        match result {
            Ok(result) => match result {
                Ok(msg) => match msg {
                    instance::OkResult::Message(s) => {
                        let _ = socket.send(Message::Text(s)).await;
                    }
                    _ => {}
                },
                Err(msg) => {
                    let _ = socket
                        .send(Message::Text(format!("CAIManager: {}", msg)))
                        .await;
                }
            },
            Err(_) => {}
        }
    }
    // Create a channel to receive the result of the operation
    let (result_sender, result_receiver) = oneshot::channel();

    // Send a request to get the reader from the broker
    let operation = instance::IOperation {
        otype: IOperationTypes::StdoutGetReader(name),
        result: Some(result_sender),
    };

    if let Err(e) = operation_sender.send(operation).await {
        error!("Failed to send operation to broker: {}", e);
        return;
    }

    // Wait for the broker to respond with the reader
    let mut receiver = match result_receiver.await {
        Ok(result) => match result {
            Ok(a) => match a {
                instance::OkResult::Receiver(receiver) => receiver,
                _ => {
                    let _ = socket
                        .send(Message::Text(
                            json!(SrvMessage {
                                message: "Getting stdout reader failed".to_string()
                            })
                            .to_string(),
                        ))
                        .await;
                    let _ = socket.close().await;
                    return;
                }
            },
            Err(a) => {
                let _ = socket
                    .send(Message::Text(json!(SrvMessage { message: a }).to_string()))
                    .await;
                let _ = socket.close().await;
                return;
            }
        },
        _ => {
            let _ = socket
                .send(Message::Text(
                    json!(SrvMessage {
                        message: "Unexpected result from broker".to_string()
                    })
                    .to_string(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    };

    // Receive output characters from the receiver and send them to the client
    loop {
        let result = receiver.recv().await;
        match result {
            Ok(msg) => {
                if socket.send(Message::Text(msg)).await.is_err() {
                    error!("Failed to send message to client");
                    break;
                }
            }
            Err(e) => {
                let _ = socket
                    .send(Message::Text(
                        json!(SrvMessage {
                            message: e.to_string()
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket.close().await;
                return;
            }
        }
    }

    let _ = socket.close().await;
}
    