# Age Inbox CLI

A high-performance command-line utility for downloading and uploading payloads to an [Age Inbox API](https://github.com/TODO) server.
It supports concurrent chunked downloads directly to disk and recursive uploads for folder sync workflows.

## Features

- **Blazing Fast**: Uses Tokio and asynchronous futures to process concurrent stream downloads.
- **Secure by Default**: Erases the vault password from RAM automatically upon unlocking the session utilizing `zeroize`.
- **Flexible Verification**: Provides support for TLS Certificate Pinning on untrusted connections.
- **Auto-Deletion**: Safely remove files from the server vault upon successful complete download.
- **Upload Modes**: Upload one file or an entire folder recursively.
- **Local Cleanup Option**: Optionally delete local files after each successful upload.

---

## Installation

Ensure you have Rust 1.94 (Edition 2021) installed:
```bash
cargo build --release
```
The compiled binary will be placed at `./target/release/age-inbox-cli`

### Docker

You can run the release image directly:
```bash
docker run -it --rm ghcr.io/cypherbits/age-inbox-cli:latest --help
```

---

## Usage

You can start the tool by invoking `age-inbox-cli` with a subcommand:

```bash
age-inbox-cli --vault myvault download --dest ./downloads
```

### Prompting Passwords

By default, the CLI will display an interactive, hidden prompt in your terminal asking for the vault decryption password.
For unattended environments or CI/CD pipelines, you can provide the password securely by exporting the `AGE_INBOX_PASSWORD` environment variable.

```bash
AGE_INBOX_PASSWORD="super-secret-password" age-inbox-cli --vault myvault download --dest ./downloads
```

## Command Line Arguments

Global arguments:

| Argument | Description | Default | Required |
|----------|-------------|---------|----------|
| `--server` | The base URL of the Age Inbox REST API. | `http://127.0.0.1:3000` | No |
| `--vault` | Name of the vault to connect. | None | **Yes** |
| `--pin-cert` | Optional path to a `.pem` public key for HTTPS certificate pinning. | None | No |

Download subcommand (`download`) arguments:

| Argument | Description | Default | Required |
|----------|-------------|---------|----------|
| `--dest` | Destination directory to save downloaded files. | None | **Yes** |
| `--file` | Optional single file path in vault. If omitted, downloads full vault. | None | No |
| `--delete` | If present, file(s) are deleted from server after download. | None | No |

Upload subcommand (`upload`) arguments:

| Argument | Description | Default | Required |
|----------|-------------|---------|----------|
| `--file` | Upload one local file. Mutually exclusive with `--folder`. | None | One of `--file` or `--folder` |
| `--folder` | Upload all files in a local folder recursively. Mutually exclusive with `--file`. | None | One of `--file` or `--folder` |
| `--delete-after-upload` | If present, deletes each local file after successful upload. | None | No |

---

## Examples

**1. Downloading all files from a vault.**
```bash
age-inbox-cli --vault test-env download --dest ./output
```

**2. Downloading one file from a vault.**
```bash
age-inbox-cli --vault test-env download --dest ./output --file reports/summary.pdf.age
```

**3. Downloading from a secure remote and clearing the vault.**
```bash
age-inbox-cli --server https://api.my-domain.com --vault client-x download --dest /var/data/client --delete
```

**4. Uploading one local file.**
```bash
age-inbox-cli --vault client-x upload --file ./to-send/report.csv
```

**5. Uploading a full folder recursively and deleting local files.**
```bash
age-inbox-cli --vault client-x upload --folder ./batch --delete-after-upload
```

**6. Utilizing TLS Pinning on self-signed domains.**
```bash
age-inbox-cli --server https://192.168.1.150:3000 --vault test download --dest ./out --pin-cert ./server.pem
```
