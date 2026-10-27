//! Java Edition Server List Ping and server icon caching.
//!
//! This module speaks the status protocol directly instead of depending on a
//! Minecraft protocol crate. It is intentionally limited to status queries; it
//! does not authenticate or join servers.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcl_domain::ServerAddress;
use serde::Deserialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
};
use tokio_util::sync::CancellationToken;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_MOTD_CHARS: usize = 4096;
const MAX_CHAT_DEPTH: usize = 16;
const MAX_ICON_BYTES: usize = 256 * 1024;
const SERVER_ICON_SIZE: u32 = 64;

/// A parsed Java Edition status response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub motd: String,
    pub online_players: Option<u32>,
    pub max_players: Option<u32>,
    pub version_name: Option<String>,
    pub protocol: Option<i32>,
    pub latency: Duration,
    pub icon_png: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ServerStatusClient {
    timeout: Duration,
    max_response_bytes: usize,
}

impl Default for ServerStatusClient {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

impl ServerStatusClient {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::default()
        }
    }

    /// Queries one server, returning an error for DNS, connection, timeout,
    /// protocol, or malformed-response failures.
    pub async fn ping(
        &self,
        address: &ServerAddress,
        cancellation: &CancellationToken,
    ) -> Result<ServerStatus, ServerPingError> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(ServerPingError::Cancelled),
            result = tokio::time::timeout(self.timeout, self.ping_inner(address)) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err(ServerPingError::Timeout),
                }
            }
        }
    }

    async fn ping_inner(&self, address: &ServerAddress) -> Result<ServerStatus, ServerPingError> {
        let sockets = lookup_host((address.host(), address.port()))
            .await
            .map_err(|source| ServerPingError::Resolve {
                host: address.host().to_owned(),
                source,
            })?
            .collect::<Vec<_>>();
        if sockets.is_empty() {
            return Err(ServerPingError::Resolve {
                host: address.host().to_owned(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no address records returned",
                ),
            });
        }
        let mut last_error = None;
        let mut stream = None;
        for socket in sockets {
            match TcpStream::connect(socket).await {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(source) => last_error = Some(source),
            }
        }
        let mut stream = stream.ok_or_else(|| ServerPingError::Connect {
            address: address.to_string(),
            source: last_error.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no address records returned")
            }),
        })?;
        stream.set_nodelay(true).map_err(ServerPingError::Io)?;

        let started = std::time::Instant::now();
        let mut handshake = Vec::new();
        write_varint(&mut handshake, 0);
        write_varint(&mut handshake, -1);
        write_string(&mut handshake, address.host());
        handshake.extend_from_slice(&address.port().to_be_bytes());
        write_varint(&mut handshake, 1);
        write_packet(&mut stream, &handshake).await?;
        write_packet(&mut stream, &[0]).await?;

        let mut response = read_packet(&mut stream, self.max_response_bytes).await?;
        let packet_id = read_varint_from_slice(&mut response)?;
        if packet_id != 0 {
            return Err(ServerPingError::Protocol(format!(
                "unexpected status packet id {packet_id}"
            )));
        }
        let json = read_string_from_slice(&mut response)?;
        let latency = started.elapsed();
        let raw: RawStatus = serde_json::from_str(&json)?;
        Ok(ServerStatus {
            motd: flatten_description(raw.description.as_ref()),
            online_players: raw.players.as_ref().and_then(|players| players.online),
            max_players: raw.players.as_ref().and_then(|players| players.max),
            version_name: raw
                .version
                .as_ref()
                .and_then(|version| version.name.clone()),
            protocol: raw.version.as_ref().and_then(|version| version.protocol),
            latency,
            icon_png: parse_favicon(raw.favicon.as_deref())?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawStatus {
    #[serde(default)]
    description: Option<Value>,
    #[serde(default)]
    players: Option<RawPlayers>,
    #[serde(default)]
    version: Option<RawVersion>,
    #[serde(default)]
    favicon: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPlayers {
    #[serde(default)]
    online: Option<u32>,
    #[serde(default)]
    max: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawVersion {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    protocol: Option<i32>,
}

/// Caches server favicons by SHA-1 so UI image elements can reuse local files.
#[derive(Debug, Clone)]
pub struct ServerIconCache {
    directory: PathBuf,
}

impl ServerIconCache {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Writes the icon atomically unless an identical icon is already cached.
    pub async fn ensure(&self, png: &[u8]) -> Result<PathBuf, ServerPingError> {
        let path = self.path_for(png);
        if tokio::fs::metadata(&path)
            .await
            .map(|metadata| metadata.len() == png.len() as u64)
            .unwrap_or(false)
        {
            return Ok(path);
        }
        let directory = self.directory.clone();
        let path_for_write = path.clone();
        let bytes = png.to_vec();
        tokio::task::spawn_blocking(move || write_icon(&directory, &path_for_write, &bytes))
            .await
            .map_err(|error| {
                ServerPingError::Protocol(format!("icon cache task failed: {error}"))
            })??;
        Ok(path)
    }

    pub fn path_for(&self, png: &[u8]) -> PathBuf {
        let hash = hex::encode(Sha1::digest(png));
        self.directory.join(format!("{hash}.png"))
    }
}

fn write_icon(directory: &Path, path: &Path, bytes: &[u8]) -> Result<(), ServerPingError> {
    std::fs::create_dir_all(directory).map_err(|source| ServerPingError::Cache {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut temporary =
        NamedTempFile::new_in(directory).map_err(|source| ServerPingError::Cache {
            path: directory.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(bytes)
        .map_err(|source| ServerPingError::Cache {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| ServerPingError::Cache {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(ServerPingError::Cache {
            path: path.to_path_buf(),
            source: error.error,
        }),
    }
}

/// Reads a status response packet with a bounded length prefix.
async fn read_packet(
    stream: &mut TcpStream,
    max_response_bytes: usize,
) -> Result<Vec<u8>, ServerPingError> {
    let length = read_varint_async(stream).await?;
    if length <= 0 {
        return Err(ServerPingError::Protocol(
            "status packet has an invalid length".to_owned(),
        ));
    }
    let length = length as usize;
    if length > max_response_bytes {
        return Err(ServerPingError::ResponseTooLarge(max_response_bytes));
    }
    let mut packet = vec![0; length];
    stream.read_exact(&mut packet).await?;
    Ok(packet)
}

async fn write_packet(stream: &mut TcpStream, payload: &[u8]) -> Result<(), ServerPingError> {
    let mut packet = Vec::with_capacity(payload.len() + 5);
    write_varint(&mut packet, payload.len() as i32);
    packet.extend_from_slice(payload);
    stream.write_all(&packet).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_varint_async(stream: &mut TcpStream) -> Result<i32, ServerPingError> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        value |= u32::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err(ServerPingError::Protocol(
        "VarInt exceeds five bytes".to_owned(),
    ))
}

fn write_varint(output: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint_from_slice(input: &mut Vec<u8>) -> Result<i32, ServerPingError> {
    let mut value = 0u32;
    for (index, shift) in (0..35).step_by(7).enumerate() {
        let Some(byte) = input.first().copied() else {
            return Err(ServerPingError::Protocol(
                "truncated VarInt in status packet".to_owned(),
            ));
        };
        input.drain(..1);
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value as i32);
        }
        if index == 4 {
            break;
        }
    }
    Err(ServerPingError::Protocol(
        "VarInt exceeds five bytes".to_owned(),
    ))
}

fn write_string(output: &mut Vec<u8>, value: &str) {
    write_varint(output, value.len() as i32);
    output.extend_from_slice(value.as_bytes());
}

fn read_string_from_slice(input: &mut Vec<u8>) -> Result<String, ServerPingError> {
    let length = read_varint_from_slice(input)?;
    if length < 0 {
        return Err(ServerPingError::Protocol(
            "status string has a negative length".to_owned(),
        ));
    }
    let length = length as usize;
    if length > input.len() {
        return Err(ServerPingError::Protocol(
            "truncated status string".to_owned(),
        ));
    }
    let bytes: Vec<u8> = input.drain(..length).collect();
    String::from_utf8(bytes)
        .map_err(|error| ServerPingError::Protocol(format!("status string is not UTF-8: {error}")))
}

fn flatten_description(description: Option<&Value>) -> String {
    let mut output = String::new();
    if let Some(description) = description {
        flatten_component(description, &mut output, 0);
    }
    strip_formatting(&output)
}

fn flatten_component(value: &Value, output: &mut String, depth: usize) {
    if depth > MAX_CHAT_DEPTH || output.chars().count() >= MAX_MOTD_CHARS {
        return;
    }
    match value {
        Value::String(text) => output.push_str(text),
        Value::Array(values) => {
            for value in values {
                flatten_component(value, output, depth + 1);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                output.push_str(text);
            } else if let Some(translate) = object.get("translate").and_then(Value::as_str) {
                output.push_str(translate);
            }
            if let Some(extra) = object.get("extra") {
                flatten_component(extra, output, depth + 1);
            }
            if let Some(values) = object.get("with").and_then(Value::as_array) {
                for value in values {
                    flatten_component(value, output, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn strip_formatting(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '§' {
            characters.next();
        } else {
            output.push(character);
        }
        if output.chars().count() >= MAX_MOTD_CHARS {
            break;
        }
    }
    output
}

fn parse_favicon(favicon: Option<&str>) -> Result<Option<Vec<u8>>, ServerPingError> {
    let Some(favicon) = favicon else {
        return Ok(None);
    };
    let Some((metadata, payload)) = favicon.split_once(',') else {
        return Err(ServerPingError::InvalidIcon);
    };
    if !metadata.eq_ignore_ascii_case("data:image/png;base64")
        || payload.len() > MAX_ICON_BYTES * 4 / 3 + 4
    {
        return Err(ServerPingError::InvalidIcon);
    }
    let bytes = BASE64
        .decode(payload)
        .map_err(|_| ServerPingError::InvalidIcon)?;
    if bytes.len() > MAX_ICON_BYTES || !is_valid_icon(&bytes) {
        return Err(ServerPingError::InvalidIcon);
    }
    Ok(Some(bytes))
}

fn is_valid_icon(bytes: &[u8]) -> bool {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    bytes.len() >= 24
        && bytes.starts_with(PNG_SIGNATURE)
        && &bytes[12..16] == b"IHDR"
        && u32::from_be_bytes(bytes[16..20].try_into().unwrap_or_default()) == SERVER_ICON_SIZE
        && u32::from_be_bytes(bytes[20..24].try_into().unwrap_or_default()) == SERVER_ICON_SIZE
}

#[derive(Debug, Error)]
pub enum ServerPingError {
    #[error("server ping was cancelled")]
    Cancelled,
    #[error("server did not respond before the timeout")]
    Timeout,
    #[error("failed to resolve server host `{host}`: {source}")]
    Resolve {
        host: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to connect to server `{address}`: {source}")]
    Connect {
        address: String,
        #[source]
        source: std::io::Error,
    },
    #[error("server I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("server sent an invalid response: {0}")]
    Protocol(String),
    #[error("server response exceeded the {0}-byte limit")]
    ResponseTooLarge(usize),
    #[error("server status JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("server icon is not a valid 64x64 PNG data URI")]
    InvalidIcon,
    #[error("failed to write server icon cache at `{path}`: {source}")]
    Cache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::net::TcpListener;

    fn encode_packet(payload: &[u8]) -> Vec<u8> {
        let mut packet = Vec::new();
        write_varint(&mut packet, payload.len() as i32);
        packet.extend_from_slice(payload);
        packet
    }

    fn encode_string(output: &mut Vec<u8>, value: &str) {
        write_string(output, value);
    }

    fn status_payload(status: serde_json::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        write_varint(&mut payload, 0);
        encode_string(&mut payload, &status.to_string());
        encode_packet(&payload)
    }

    async fn read_test_packet(stream: &mut TcpStream) -> Vec<u8> {
        let length = read_varint_async(stream).await.unwrap();
        let mut packet = vec![0; length as usize];
        stream.read_exact(&mut packet).await.unwrap();
        packet
    }

    #[test]
    fn varints_round_trip_through_slices() {
        for value in [0, 1, 127, 128, 25565, -1] {
            let mut encoded = Vec::new();
            write_varint(&mut encoded, value);
            let mut encoded_for_read = encoded.clone();
            assert_eq!(
                read_varint_from_slice(&mut encoded_for_read).unwrap(),
                value
            );
        }
    }

    #[test]
    fn flattens_nested_motd_and_strips_formatting() {
        let description = json!({
            "text": "§aHello ",
            "extra": [
                {"text": "world"},
                {"text": " §l!", "extra": [{"text": " nested"}]}
            ]
        });
        assert_eq!(
            flatten_description(Some(&description)),
            "Hello world ! nested"
        );
        assert_eq!(
            flatten_description(Some(&Value::String("plain".to_owned()))),
            "plain"
        );
    }

    #[test]
    fn validates_icon_data_uri() {
        let png = test_icon();
        let data_uri = format!("data:image/png;base64,{}", BASE64.encode(&png));
        assert_eq!(parse_favicon(Some(&data_uri)).unwrap(), Some(png));
        assert!(parse_favicon(Some("data:image/png;base64,not-base64")).is_err());
        assert!(parse_favicon(Some("http://example.com/icon.png")).is_err());
    }

    fn test_icon() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([1, 2, 3, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[tokio::test]
    async fn pings_a_mock_server_and_returns_status() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let png = test_icon();
        let favicon = format!("data:image/png;base64,{}", BASE64.encode(&png));
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let handshake = read_test_packet(&mut stream).await;
            assert_eq!(handshake[0], 0);
            let request = read_test_packet(&mut stream).await;
            assert_eq!(request, vec![0]);
            let status = status_payload(json!({
                "version": {"name": "1.21.1", "protocol": 767},
                "players": {"online": 3, "max": 20},
                "description": {"text": "Test ", "extra": [{"text": "Server"}]},
                "favicon": favicon
            }));
            stream.write_all(&status).await.unwrap();
        });
        let server_address = ServerAddress::parse(&address.to_string()).unwrap();
        let status = ServerStatusClient::default()
            .ping(&server_address, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(status.motd, "Test Server");
        assert_eq!(status.online_players, Some(3));
        assert_eq!(status.max_players, Some(20));
        assert_eq!(status.version_name.as_deref(), Some("1.21.1"));
        assert_eq!(status.protocol, Some(767));
        assert_eq!(status.icon_png.as_deref(), Some(png.as_slice()));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn times_out_when_the_server_does_not_answer() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let server_address = ServerAddress::parse(&address.to_string()).unwrap();
        let error = ServerStatusClient::new(Duration::from_millis(100))
            .ping(&server_address, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(error, ServerPingError::Timeout), "{error}");
        server.abort();
    }

    #[tokio::test]
    async fn caches_icon_by_content_hash() {
        let temporary = tempfile::TempDir::new().unwrap();
        let cache = ServerIconCache::new(temporary.path());
        let png = test_icon();
        let first = cache.ensure(&png).await.unwrap();
        let second = cache.ensure(&png).await.unwrap();
        assert_eq!(first, second);
        assert!(first.exists());
        assert_eq!(std::fs::read(&first).unwrap(), png);
    }

    #[ignore = "pings the server named by RMCL_TEST_SERVER"]
    #[tokio::test]
    async fn pings_a_real_server() {
        let raw = std::env::var("RMCL_TEST_SERVER")
            .expect("set RMCL_TEST_SERVER to host[:port] before running this smoke test");
        let address = ServerAddress::parse(&raw).unwrap();
        let status = ServerStatusClient::default()
            .ping(&address, &CancellationToken::new())
            .await
            .unwrap();
        assert!(status.latency > Duration::ZERO);
    }
}
