use anyhow::{Context, Result};
use clap::Parser;
use futures_util::stream::StreamExt;
use reqwest::{Certificate, Client};
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

    /// Destination Folder
    #[arg(long)]
    pub dest: PathBuf,

    /// Delete files on the server after downloading
    #[arg(long)]
    pub delete: bool,

    /// Optional public key PEM file for HTTPS pinning
    #[arg(long)]
    pub pin_cert: Option<PathBuf>,
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
    // Setup HTTP Client
    let mut client_builder = Client::builder();
    if let Some(cert_path) = &args.pin_cert {
        let cert_bytes = fs::read(cert_path)
            .await
            .context("Failed to read pin-cert file")?;
        let cert = Certificate::from_pem(&cert_bytes)
            .context("Failed to parse PEM certificate")?;
        client_builder = client_builder.add_root_certificate(cert);
    }

    let client = client_builder.build().context("Failed to build HTTP client")?;
    let base_url = args.server.trim_end_matches('/');

    // 1. Check if vault exists
    info!("Checking if vault '{}' exists...", args.vault);
    let raw_list_url = format!("{}/inbox/{}/raw/list", base_url, args.vault);
    let res = client.get(&raw_list_url).send().await?;
    if !res.status().is_success() {
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("Vault '{}' not found on server", args.vault);
        }
        anyhow::bail!("Failed to query vault status: {}", res.status());
    }

    // 2. Prompt for password or use env var
    let password_str = match env::var("AGE_INBOX_PASSWORD") {
        Ok(pass) => {
            info!("Using password from AGE_INBOX_PASSWORD environment variable.");
            pass
        }
        Err(_) => rpassword::prompt_password("Enter vault password: ")
            .context("Failed to read password")?,
    };
    let password = Zeroizing::new(password_str);

    // 3. Unlock Vault
    info!("Unlocking vault...");
    let unlock_url = format!("{}/inbox/{}/unlock", base_url, args.vault);
    let unlock_req = json!({ "password": &*password });
    let res = client.post(&unlock_url).json(&unlock_req).send().await?;

    if !res.status().is_success() {
        anyhow::bail!("Failed to unlock vault: {:?}", res.status());
    }

    // Process downloads. If it fails or succeeds, we must lock the vault afterwards.
    let result = run_downloads(&args, &client, base_url).await;

    // 6. Lock Vault
    info!("Locking vault...");
    let lock_url = format!("{}/inbox/{}/lock", base_url, args.vault);
    let _ = client.post(&lock_url).send().await; // Ignore failure, best-effort lock

    result
}

async fn run_downloads(args: &Args, client: &Client, base_url: &str) -> Result<()> {
    // 4. List Files
    info!("Fetching file list...");
    let list_url = format!("{}/inbox/{}/list", base_url, args.vault);
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
    if !args.dest.exists() {
        fs::create_dir_all(&args.dest)
            .await
            .context("Failed to create destination directory")?;
    }

    // 5. Download concurrently
    let concurrency = 8;
    let client_ref = client;

    futures_util::stream::iter(files)
        .for_each_concurrent(concurrency, |file| async move {
            if let Err(e) = download_file(
                client_ref,
                base_url,
                &args.vault,
                &args.dest,
                args.delete,
                &file,
            )
            .await
            {
                error!("Failed to process file '{}': {}", file.path, e);
            }
        })
        .await;

    info!("All downloads completed.");
    Ok(())
}

async fn download_file(
    client: &Client,
    base_url: &str,
    vault: &str,
    dest_dir: &Path,
    delete: bool,
    file: &ListedFile,
) -> Result<()> {
    let download_url = format!("{}/inbox/{}/download/{}", base_url, vault, file.path);
    let res = client.get(&download_url).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("Download request failed with status {}", res.status());
    }

    let mut stream = res.bytes_stream();

    // Preserve the original path structure (e.g., 'subfolders')
    let file_path = Path::new(&file.path);
    let parent_dir = file_path.parent().unwrap_or_else(|| Path::new(""));

    // Choose the filename to save as. Use metadata 'filename' if present, otherwise strip '.age'.
    let file_name = if let Some(ref fname) = file.filename {
        fname.clone()
    } else {
        file.path
            .strip_suffix(".age")
            .unwrap_or(&file.path)
            .to_string()
    };

    let target_path = dest_dir.join(parent_dir).join(&file_name);

    if let Some(parent) = target_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).await?;
        }
    }

    let mut out_file = File::create(&target_path).await?;
    info!("Downloading: {} -> {}", file.path, target_path.display());

    while let Some(chunk) = stream.next().await {
        let data = chunk?;
        out_file.write_all(&data).await?;
    }
    out_file.flush().await?;

    if delete {
        info!("Deleting file from server: {}", file.path);
        let delete_url = format!("{}/inbox/{}/delete/{}", base_url, vault, file.path);
        let del_res = client.delete(&delete_url).send().await?;
        if !del_res.status().is_success() {
            warn!(
                "Failed to delete {} from server: {}",
                file.path,
                del_res.status()
            );
        }
    }

    Ok(())
}
