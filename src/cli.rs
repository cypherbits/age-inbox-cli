use anyhow::{Context, Result};
use clap::{Args as ClapArgs, Parser, Subcommand};
use futures_util::stream::StreamExt;
use reqwest::{Certificate, Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Server URL
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    pub server: String,

    /// Vault Name
    #[arg(long)]
    pub vault: String,

    /// Optional public key PEM file for HTTPS pinning
    #[arg(long)]
    pub pin_cert: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Download one file or all files from a vault
    Download(DownloadArgs),
    /// Upload one file or all files from a folder
    Upload(UploadArgs),
}

#[derive(ClapArgs, Debug)]
pub struct DownloadArgs {
    /// Destination folder
    #[arg(long)]
    pub dest: PathBuf,

    /// Optional single file path in vault (downloads all files if omitted)
    #[arg(long)]
    pub file: Option<String>,

    /// Delete files on the server after downloading
    #[arg(long)]
    pub delete: bool,
}

#[derive(ClapArgs, Debug)]
pub struct UploadArgs {
    /// Local file to upload
    #[arg(long, conflicts_with = "folder")]
    pub file: Option<PathBuf>,

    /// Local folder to upload recursively
    #[arg(long, conflicts_with = "file")]
    pub folder: Option<PathBuf>,

    /// Delete local file after successful upload
    #[arg(long)]
    pub delete_after_upload: bool,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ListedFile {
    pub path: String,
    pub filename: Option<String>,
    #[allow(dead_code)]
    pub origin: Option<String>,
    #[allow(dead_code)]
    pub size: u64,
}

pub async fn run_cli(args: Args) -> Result<()> {
    let client = build_client(args.pin_cert.as_ref()).await?;
    let base_url = args.server.trim_end_matches('/');

    ensure_vault_exists(&client, base_url, &args.vault).await?;

    match &args.command {
        Command::Download(download_args) => {
            run_download_flow(&args, download_args, &client, base_url).await
        }
        Command::Upload(upload_args) => run_uploads(&args, upload_args, &client, base_url).await,
    }
}

async fn build_client(pin_cert: Option<&PathBuf>) -> Result<Client> {
    let mut client_builder = Client::builder();
    if let Some(cert_path) = pin_cert {
        let cert_bytes = fs::read(cert_path)
            .await
            .context("Failed to read pin-cert file")?;
        let cert = Certificate::from_pem(&cert_bytes).context("Failed to parse PEM certificate")?;
        client_builder = client_builder.add_root_certificate(cert);
    }

    client_builder
        .build()
        .context("Failed to build HTTP client")
}

async fn ensure_vault_exists(client: &Client, base_url: &str, vault: &str) -> Result<()> {
    info!("Checking if vault '{}' exists...", vault);
    let config_url = vault_url(base_url, vault, "config")?;
    let res = client.get(&config_url).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("Vault '{}' not found on server", vault);
    }
    if !res.status().is_success() {
        anyhow::bail!("Failed to query vault status: {}", res.status());
    }

    Ok(())
}

async fn run_download_flow(
    args: &Args,
    download_args: &DownloadArgs,
    client: &Client,
    base_url: &str,
) -> Result<()> {
    unlock_vault(args, client, base_url).await?;

    let result = run_downloads(args, download_args, client, base_url).await;

    let _ = lock_vault(args, client, base_url).await;
    result
}

async fn unlock_vault(args: &Args, client: &Client, base_url: &str) -> Result<()> {
    let password_str = match env::var("AGE_INBOX_PASSWORD") {
        Ok(pass) => {
            info!("Using password from AGE_INBOX_PASSWORD environment variable.");
            pass
        }
        Err(_) => rpassword::prompt_password("Enter vault password: ")
            .context("Failed to read password")?,
    };
    let password = Zeroizing::new(password_str);

    info!("Unlocking vault...");
    let unlock_url = vault_url(base_url, &args.vault, "unlock")?;
    let unlock_req = json!({ "password": &*password });
    let res = client.post(&unlock_url).json(&unlock_req).send().await?;

    if !res.status().is_success() {
        anyhow::bail!("Failed to unlock vault: {:?}", res.status());
    }

    Ok(())
}

async fn lock_vault(args: &Args, client: &Client, base_url: &str) -> Result<()> {
    info!("Locking vault...");
    let lock_url = vault_url(base_url, &args.vault, "lock")?;
    client.post(&lock_url).send().await?;
    Ok(())
}

async fn run_downloads(
    args: &Args,
    download_args: &DownloadArgs,
    client: &Client,
    base_url: &str,
) -> Result<()> {
    if !download_args.dest.exists() {
        fs::create_dir_all(&download_args.dest)
            .await
            .context("Failed to create destination directory")?;
    }

    if let Some(file_path) = &download_args.file {
        info!("Downloading single file: {}", file_path);
        download_file(
            client,
            base_url,
            &args.vault,
            &download_args.dest,
            download_args.delete,
            file_path,
            None,
        )
        .await?;
        info!("Single file download completed.");
        return Ok(());
    }

    info!("Fetching file list...");
    let list_url = vault_url(base_url, &args.vault, "list")?;
    let res = client.get(&list_url).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("Failed to fetch file list: {}", res.status());
    }

    let files: Vec<ListedFile> = res.json().await?;
    if files.is_empty() {
        info!("Vault is empty.");
        return Ok(());
    }

    info!("Found {} files to download.", files.len());

    let concurrency = 8;
    let client_ref = client;
    let vault = args.vault.clone();
    let dest = download_args.dest.clone();
    let delete = download_args.delete;

    futures_util::stream::iter(files)
        .for_each_concurrent(concurrency, |file| {
            let vault = vault.clone();
            let dest = dest.clone();
            async move {
                if let Err(e) = download_file(
                    client_ref,
                    base_url,
                    &vault,
                    &dest,
                    delete,
                    &file.path,
                    file.filename.as_deref(),
                )
                .await
                {
                    error!("Failed to process file '{}': {}", file.path, e);
                }
            }
        })
        .await;

    info!("All downloads completed.");
    Ok(())
}

async fn run_uploads(
    args: &Args,
    upload_args: &UploadArgs,
    client: &Client,
    base_url: &str,
) -> Result<()> {
    match (&upload_args.file, &upload_args.folder) {
        (Some(file_path), None) => {
            upload_local_file(client, base_url, &args.vault, file_path, Path::new("")).await?;
            if upload_args.delete_after_upload {
                fs::remove_file(file_path).await.with_context(|| {
                    format!("Failed to delete local file {}", file_path.display())
                })?;
                info!("Deleted local file after upload: {}", file_path.display());
            }
            Ok(())
        }
        (None, Some(folder_path)) => {
            let files = collect_local_files(folder_path).await?;
            if files.is_empty() {
                info!("Folder is empty. Nothing to upload.");
                return Ok(());
            }

            info!("Uploading {} files from folder...", files.len());
            for file_path in files {
                let rel_path = file_path.strip_prefix(folder_path).with_context(|| {
                    format!(
                        "Failed to compute relative path for {}",
                        file_path.display()
                    )
                })?;

                let remote_dir = rel_path.parent().unwrap_or_else(|| Path::new(""));
                upload_local_file(client, base_url, &args.vault, &file_path, remote_dir).await?;

                if upload_args.delete_after_upload {
                    fs::remove_file(&file_path).await.with_context(|| {
                        format!("Failed to delete local file {}", file_path.display())
                    })?;
                    info!("Deleted local file after upload: {}", file_path.display());
                }
            }

            info!("Folder upload completed.");
            Ok(())
        }
        _ => anyhow::bail!("Specify exactly one upload source: --file or --folder"),
    }
}

async fn collect_local_files(folder: &Path) -> Result<Vec<PathBuf>> {
    if !folder.exists() {
        anyhow::bail!("Folder does not exist: {}", folder.display());
    }
    if !folder.is_dir() {
        anyhow::bail!("Path is not a directory: {}", folder.display());
    }

    let mut files = Vec::new();
    let mut pending_dirs = vec![folder.to_path_buf()];

    while let Some(current_dir) = pending_dirs.pop() {
        let mut entries = fs::read_dir(&current_dir)
            .await
            .with_context(|| format!("Failed to read directory {}", current_dir.display()))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                pending_dirs.push(path);
            } else if metadata.is_file() {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

async fn upload_local_file(
    client: &Client,
    base_url: &str,
    vault: &str,
    local_file: &Path,
    remote_dir: &Path,
) -> Result<()> {
    if !local_file.exists() {
        anyhow::bail!("Local file does not exist: {}", local_file.display());
    }

    let filename = local_file
        .file_name()
        .and_then(|f| f.to_str())
        .context("Failed to resolve upload filename")?
        .to_string();

    let content = fs::read(local_file)
        .await
        .with_context(|| format!("Failed to read local file {}", local_file.display()))?;

    let form = reqwest::multipart::Form::new()
        .text("filename", filename.clone())
        .text("origin", "age-inbox-cli")
        .part(
            "file",
            reqwest::multipart::Part::bytes(content)
                .file_name(filename)
                .mime_str("application/octet-stream")?,
        );

    let remote_dir_url = path_to_url_path(remote_dir);
    let upload_url = if remote_dir_url.is_empty() {
        vault_url(base_url, vault, "upload")?
    } else {
        vault_url_with_path(base_url, vault, "upload", &remote_dir_url)?
    };

    info!(
        "Uploading {} -> {}",
        local_file.display(),
        if remote_dir_url.is_empty() {
            "/"
        } else {
            &remote_dir_url
        }
    );

    let res = client.post(&upload_url).multipart(form).send().await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
        anyhow::bail!(
            "Upload failed for {}: {} - {}",
            local_file.display(),
            status,
            body
        );
    }

    Ok(())
}

fn path_to_url_path(path: &Path) -> String {
    path.components()
        .filter_map(|c| {
            let seg = c.as_os_str().to_string_lossy();
            if seg == "." || seg.is_empty() {
                None
            } else {
                Some(seg.replace('\\', "/"))
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn vault_url(base_url: &str, vault: &str, operation: &str) -> Result<String> {
    let mut url = Url::parse(base_url).context("Invalid server URL")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Server URL cannot be a base URL"))?;
        segments.push("inbox");
        segments.push(vault);
        segments.push(operation);
    }
    Ok(url.into())
}

fn vault_url_with_path(base_url: &str, vault: &str, operation: &str, path: &str) -> Result<String> {
    let mut url = Url::parse(base_url).context("Invalid server URL")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Server URL cannot be a base URL"))?;
        segments.push("inbox");
        segments.push(vault);
        segments.push(operation);
        for segment in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
            segments.push(segment);
        }
    }
    Ok(url.into())
}

/// Parses the `Content-Disposition` header and returns the `filename=` value when present.
/// Example: `attachment; filename="report.pdf"` → `Some("report.pdf")`
fn parse_content_disposition_filename(header: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        if part.to_lowercase().starts_with("filename=") {
            let val = part["filename=".len()..].trim_matches('"');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

async fn download_file(
    client: &Client,
    base_url: &str,
    vault: &str,
    dest_dir: &Path,
    delete: bool,
    remote_path: &str,
    metadata_filename: Option<&str>,
) -> Result<()> {
    let normalized_remote_path = remote_path.replace('\\', "/");
    let download_url = vault_url_with_path(base_url, vault, "download", &normalized_remote_path)?;
    let res = client.get(&download_url).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("Download request failed with status {}", res.status());
    }

    // Prefer server-provided filename when available.
    let filename_from_header: Option<String> = res
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_disposition_filename);

    let mut stream = res.bytes_stream();

    let file_path = Path::new(&normalized_remote_path);
    let parent_dir = file_path.parent().unwrap_or_else(|| Path::new(""));

    let file_name = if let Some(fname) = metadata_filename {
        fname.to_string()
    } else if let Some(fname) = filename_from_header {
        fname
    } else {
        let base_name = file_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("downloaded.bin");
        base_name
            .strip_suffix(".age")
            .unwrap_or(base_name)
            .to_string()
    };

    let target_path = dest_dir.join(parent_dir).join(&file_name);

    if let Some(parent) = target_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).await?;
        }
    }

    let mut out_file = File::create(&target_path).await?;
    info!(
        "Downloading: {} -> {}",
        normalized_remote_path,
        target_path.display()
    );

    while let Some(chunk) = stream.next().await {
        let data = chunk?;
        out_file.write_all(&data).await?;
    }
    out_file.flush().await?;

    if delete {
        info!("Deleting file from server: {}", normalized_remote_path);
        let delete_url = vault_url_with_path(base_url, vault, "delete", &normalized_remote_path)?;
        let del_res = client.delete(&delete_url).send().await?;
        if !del_res.status().is_success() {
            warn!(
                "Failed to delete {} from server: {}",
                normalized_remote_path,
                del_res.status()
            );
        }
    }

    Ok(())
}

