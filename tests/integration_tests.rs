use age_inbox_cli::cli::{Args, Command, DownloadArgs, ListedFile, UploadArgs};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Clone)]
struct DownloadState {
    files: Arc<HashMap<String, Vec<u8>>>,
    deleted: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct UploadState {
    uploaded_paths: Arc<Mutex<Vec<String>>>,
}

async fn mock_raw_list() -> impl IntoResponse {
    (StatusCode::OK, Json(vec![] as Vec<String>))
}

async fn mock_unlock() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"success": true})))
}

async fn mock_lock() -> impl IntoResponse {
    StatusCode::OK
}

async fn mock_list(State(state): State<DownloadState>) -> impl IntoResponse {
    let mut files = Vec::new();
    for key in state.files.keys() {
        let filename = key
            .split('/')
            .next_back()
            .map(|name| name.strip_suffix(".age").unwrap_or(name).to_string());

        files.push(ListedFile {
            path: key.clone(),
            filename,
            origin: None,
            size: state.files.get(key).map_or(0, |v| v.len() as u64),
        });
    }

    (StatusCode::OK, Json(files))
}

async fn mock_download(
    State(state): State<DownloadState>,
    Path((_vault, filepath)): Path<(String, String)>,
) -> Response {
    if let Some(content) = state.files.get(&filepath) {
        (StatusCode::OK, content.clone()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Not Found").into_response()
    }
}

async fn mock_delete(
    State(state): State<DownloadState>,
    Path((_vault, filepath)): Path<(String, String)>,
) -> impl IntoResponse {
    state.deleted.lock().unwrap().push(filepath);
    StatusCode::OK
}

async fn mock_upload_root(
    State(state): State<UploadState>,
    Path(_vault): Path<String>,
    _body: Bytes,
) -> impl IntoResponse {
    state.uploaded_paths.lock().unwrap().push("".to_string());
    StatusCode::OK
}

async fn mock_upload_path(
    State(state): State<UploadState>,
    Path((_vault, path)): Path<(String, String)>,
    _body: Bytes,
) -> impl IntoResponse {
    state.uploaded_paths.lock().unwrap().push(path);
    StatusCode::OK
}

async fn start_download_mock_server() -> (SocketAddr, oneshot::Sender<()>, DownloadState) {
    let mut files = HashMap::new();
    files.insert(
        "test_folder/secret.txt.age".to_string(),
        b"hello world".to_vec(),
    );
    files.insert(
        "second_folder/note.txt.age".to_string(),
        b"sample note".to_vec(),
    );

    let state = DownloadState {
        files: Arc::new(files),
        deleted: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/inbox/{vault}/raw/list", get(mock_raw_list))
        .route("/inbox/{vault}/unlock", post(mock_unlock))
        .route("/inbox/{vault}/lock", post(mock_lock))
        .route("/inbox/{vault}/list", get(mock_list))
        .route("/inbox/{vault}/download/{*path}", get(mock_download))
        .route("/inbox/{vault}/delete/{*path}", delete(mock_delete))
        .with_state(state.clone());

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

    (addr, tx, state)
}

async fn start_upload_mock_server() -> (SocketAddr, oneshot::Sender<()>, UploadState) {
    let state = UploadState {
        uploaded_paths: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/inbox/{vault}/raw/list", get(mock_raw_list))
        .route("/inbox/{vault}/upload", post(mock_upload_root))
        .route("/inbox/{vault}/upload/{*path}", post(mock_upload_path))
        .with_state(state.clone());

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

    (addr, tx, state)
}

#[tokio::test]
async fn test_download_full_vault_and_delete() {
    let (addr, shutdown_tx, state) = start_download_mock_server().await;
    let server_url = format!("http://{}", addr);

    let dest_dir = tempfile::tempdir().unwrap();
    let dest_path = dest_dir.path().to_path_buf();

    env::set_var("AGE_INBOX_PASSWORD", "super-secret");

    let args = Args {
        server: server_url,
        vault: "myvault".to_string(),
        pin_cert: None,
        command: Command::Download(DownloadArgs {
            dest: dest_path.clone(),
            file: None,
            delete: true,
        }),
    };

    let result = age_inbox_cli::cli::run_cli(args).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = shutdown_tx.send(());

    assert!(result.is_ok(), "CLI execution failed: {:?}", result.err());

    let downloaded_file_1 = dest_path.join("test_folder").join("secret.txt");
    let downloaded_file_2 = dest_path.join("second_folder").join("note.txt");
    assert!(
        downloaded_file_1.exists(),
        "First downloaded file must exist"
    );
    assert!(
        downloaded_file_2.exists(),
        "Second downloaded file must exist"
    );

    let content_1 = tokio::fs::read_to_string(&downloaded_file_1).await.unwrap();
    let content_2 = tokio::fs::read_to_string(&downloaded_file_2).await.unwrap();
    assert_eq!(content_1, "hello world");
    assert_eq!(content_2, "sample note");

    let deleted = state.deleted.lock().unwrap().clone();
    assert!(deleted.contains(&"test_folder/secret.txt.age".to_string()));
    assert!(deleted.contains(&"second_folder/note.txt.age".to_string()));
}

#[tokio::test]
async fn test_download_single_file() {
    let (addr, shutdown_tx, _state) = start_download_mock_server().await;
    let server_url = format!("http://{}", addr);

    let dest_dir = tempfile::tempdir().unwrap();
    let dest_path = dest_dir.path().to_path_buf();

    env::set_var("AGE_INBOX_PASSWORD", "super-secret");

    let args = Args {
        server: server_url,
        vault: "myvault".to_string(),
        pin_cert: None,
        command: Command::Download(DownloadArgs {
            dest: dest_path.clone(),
            file: Some("test_folder/secret.txt.age".to_string()),
            delete: false,
        }),
    };

    let result = age_inbox_cli::cli::run_cli(args).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = shutdown_tx.send(());

    assert!(result.is_ok(), "CLI execution failed: {:?}", result.err());

    let downloaded_file = dest_path.join("test_folder").join("secret.txt");
    assert!(downloaded_file.exists(), "Downloaded file must exist");

    let content = tokio::fs::read_to_string(&downloaded_file).await.unwrap();
    assert_eq!(content, "hello world");
}

#[tokio::test]
async fn test_upload_single_file() {
    let (addr, shutdown_tx, state) = start_upload_mock_server().await;
    let server_url = format!("http://{}", addr);

    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("upload.txt");
    tokio::fs::write(&file_path, "upload me").await.unwrap();

    let args = Args {
        server: server_url,
        vault: "myvault".to_string(),
        pin_cert: None,
        command: Command::Upload(UploadArgs {
            file: Some(file_path.clone()),
            folder: None,
            delete_after_upload: false,
        }),
    };

    let result = age_inbox_cli::cli::run_cli(args).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = shutdown_tx.send(());

    assert!(result.is_ok(), "CLI execution failed: {:?}", result.err());
    assert!(file_path.exists(), "Local file should not be deleted");

    let uploaded = state.uploaded_paths.lock().unwrap().clone();
    assert_eq!(uploaded.len(), 1);
    assert_eq!(uploaded[0], "");
}

#[tokio::test]
async fn test_upload_folder_and_delete_local_files() {
    let (addr, shutdown_tx, state) = start_upload_mock_server().await;
    let server_url = format!("http://{}", addr);

    let temp_dir = tempfile::tempdir().unwrap();
    let folder = temp_dir.path().join("batch");
    let nested = folder.join("nested");

    tokio::fs::create_dir_all(&nested).await.unwrap();
    let root_file = folder.join("a.txt");
    let nested_file = nested.join("b.txt");
    tokio::fs::write(&root_file, "A").await.unwrap();
    tokio::fs::write(&nested_file, "B").await.unwrap();

    let args = Args {
        server: server_url,
        vault: "myvault".to_string(),
        pin_cert: None,
        command: Command::Upload(UploadArgs {
            file: None,
            folder: Some(folder.clone()),
            delete_after_upload: true,
        }),
    };

    let result = age_inbox_cli::cli::run_cli(args).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = shutdown_tx.send(());

    assert!(result.is_ok(), "CLI execution failed: {:?}", result.err());
    assert!(!root_file.exists(), "Root local file must be deleted");
    assert!(!nested_file.exists(), "Nested local file must be deleted");

    let uploaded = state.uploaded_paths.lock().unwrap().clone();
    assert_eq!(uploaded.len(), 2);
    assert!(uploaded.contains(&"".to_string()));
    assert!(uploaded.contains(&"nested".to_string()));
}
