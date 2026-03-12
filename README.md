# Age Inbox CLI

A high-performance command-line utility for downloading encrypted payloads from an [Age Inbox API](https://github.com/TODO) server. 
It supports multiple concurrent chunked streams directly to disk, ensuring maximum speed while strictly maintaining memory isolation.

## Features

- **Blazing Fast**: Uses Tokio and asynchronous futures to process concurrent stream downloads.
- **Secure by Default**: Erases the vault password from RAM automatically upon unlocking the session utilizing `zeroize`.
- **Flexible Verification**: Provides support for TLS Certificate Pinning on untrusted connections.
- **Auto-Deletion**: Safely remove files from the server vault upon successful complete download.

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
docker run -it --rm ghcr.io/YOUR_ORG/age-inbox-cli:latest --help
```

---

## Usage

You can start the tool by invoking `age-inbox-cli`:

```bash
age-inbox-cli --vault myvault --dest ./downloads
```

### Prompting Passwords

By default, the CLI will display an interactive, hidden prompt in your terminal asking for the vault decryption password.
For unattended environments or CI/CD pipelines, you can provide the password securely by exporting the `AGE_INBOX_PASSWORD` environment variable.

```bash
AGE_INBOX_PASSWORD="super-secret-password" age-inbox-cli --vault myvault --dest ./downloads
```

## Command Line Arguments

| Argument | Description | Default | Required |
|----------|-------------|---------|----------|
| `--server` | The base URL of the Age Inbox REST API. | `http://127.0.0.1:3000` | No |
| `--vault` | Name of the vault to connect and unlock. | None | **Yes** |
| `--dest` | Destination directory to save the decrypted downloaded files. | None | **Yes** |
| `--delete` | A boolean flag. If present, files will be deleted from the server upon successful download. | None | No |
| `--pin-cert` | Optional path to a `.pem` public key for HTTPS certificate pinning. | None | No |

---

## Examples

**1. Connecting to a local dashboard and retrieving files.**
```bash
age-inbox-cli --vault test-env --dest ./output
```

**2. Downloading from a secure remote and clearing the vault.**
```bash
age-inbox-cli --server https://api.my-domain.com --vault client-x --dest /var/data/client --delete
```

**3. Utilizing TLS Pinning on self-signed domains.**
```bash
age-inbox-cli --server https://192.168.1.150:3000 --vault test --dest ./out --pin-cert ./server.pem
```
