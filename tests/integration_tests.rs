use age_inbox_cli::cli::{Args, ListedFile};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

// Mock handlers

async fn mock_raw_list() -> impl IntoResponse {
    (StatusCode::OK, Json(vec![] as Vec<String>))
}

async fn mock_unlock() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"success": true})))
}

async fn mock_lock() -> impl IntoResponse {
    StatusCode::OK
}

async fn mock_list() -> impl IntoResponse {
    let files = vec![ListedFile {
        path: "test_folder/secret.txt".to_string(),
        filename: Some("secret.txt".to_string()),
        origin: None,
        size: 11,
    }];
    (StatusCode::OK, Json(files))
}

async fn mock_download(Path((_vault, filepath)): Path<(String, String)>) -> Response {
    if filepath == "test_folder/secret.txt" {
        (
            StatusCode::OK,
            "hello world".to_string(), // 11 bytes
        )
            .into_response()
    } else {
        (StatusCode::NOT_FOUND, "Not Found").into_response()
    }
}

async fn mock_delete(Path((_vault, filepath)): Path<(String, String)>) -> impl IntoResponse {
    println!("Mock server deleted: {}", filepath);
    StatusCode::OK
}

async fn start_mock_server() -> (SocketAddr, oneshot::Sender<()>) {
    let app = Router::new()
        .route("/inbox/{vault}/raw/list", get(mock_raw_list))
        .route("/inbox/{vault}/unlock", post(mock_unlock))
        .route("/inbox/{vault}/lock", post(mock_lock))
        .route("/inbox/{vault}/list", get(mock_list))
        .route("/inbox/{vault}/download/{*path}", get(mock_download))
        .route("/inbox/{vault}/delete/{*path}", delete(mock_delete));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (tx, rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                rx.await.ok();
            })
            .await;
    });

    (addr, tx)
}

#[tokio::test]
async fn test_cli_integration_download_and_delete() {
    // 1. Start mock server
    let (addr, shutdown_tx) = start_mock_server().await;
    let server_url = format!("http://{}", addr);

    // 2. Prepare temp destination folder
    let dest_dir = tempfile::tempdir().unwrap();
    let dest_path = dest_dir.path().to_path_buf();

    // 3. Inject password env var
    env::set_var("AGE_INBOX_PASSWORD", "super-secret");

    // 4. Prepare CLI Args
    let args = Args {
        server: server_url,
        vault: "myvault".to_string(),
        dest: dest_path.clone(),
        delete: true,
        pin_cert: None,
    };

    // 5. Run CLI
    let result = age_inbox_cli::cli::run_cli(args).await;
    
    // Give filesystem a moment to sync if needed
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 6. Shutdown mock server
    let _ = shutdown_tx.send(());

    // 7. Assertions
    assert!(result.is_ok(), "CLI execution failed: {:?}", result.err());

    let downloaded_file = dest_path.join("test_folder").join("secret.txt");
    assert!(downloaded_file.exists(), "Downloaded file must exist");

    let content = tokio::fs::read_to_string(&downloaded_file).await.unwrap();
    assert_eq!(content, "hello world", "File content must match");
}
