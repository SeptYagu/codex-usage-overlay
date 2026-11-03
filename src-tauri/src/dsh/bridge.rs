//! Discovery records belong to the optional overlay companion, not DSH's
//! private authentication store. Only this dedicated, user-scoped directory is
//! readable through the pairing API. Nothing here reads DSH/OAuth credentials.
use reqwest::{header::HeaderValue, Url};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

const MAX_RECORD_BYTES: usize = 16 * 1024;
pub(super) struct Record {
    pub instance_id: String,
    pub profile_id: Option<String>,
    pub origin: Url,
    pub bearer: HeaderValue,
    pub path: PathBuf,
    pub directory: PathBuf,
}

pub(super) async fn read_record_in_directory(
    path: &Path,
    directory: &Path,
) -> Result<Record, String> {
    let directory = tokio::fs::canonicalize(directory)
        .await
        .map_err(|_| "DSH companion discovery directory is unavailable")?;
    let path = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| "DSH companion discovery record is unavailable")?;
    if path.parent() != Some(directory.as_path())
        || path.extension().and_then(|s| s.to_str()) != Some("json")
    {
        return Err("Select a discovery record in the overlay companion directory".into());
    }
    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| "Cannot read DSH companion discovery record")?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| "Cannot inspect DSH companion discovery record")?;
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES as u64 {
        return Err("Invalid DSH companion discovery record size".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Cannot read DSH companion discovery record")?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err("DSH companion discovery record exceeds the size limit".into());
    }
    let mut record = parse_record(&bytes)?;
    if path.file_stem().and_then(|s| s.to_str()) != Some(record.instance_id.as_str()) {
        return Err("DSH companion discovery filename and instance identity disagree".into());
    }
    record.path = path;
    record.directory = directory;
    Ok(record)
}

pub(super) fn parse_record(bytes: &[u8]) -> Result<Record, String> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err("DSH companion discovery record exceeds the size limit".into());
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "Malformed DSH companion discovery JSON")?;
    if value["schemaVersion"] != 1 {
        return Err("Unsupported DSH companion discovery schema".into());
    }
    let id = value["instanceId"]
        .as_str()
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or("Invalid DSH companion instance identity")?;
    let origin = value["origin"]
        .as_str()
        .ok_or("Missing DSH companion origin")?;
    let parsed = Url::parse(origin).map_err(|_| "Invalid DSH companion origin")?;
    if parsed.path() != "/" || parsed.query().is_some() {
        return Err("DSH companion origin must not include a path or query".into());
    }
    let origin = super::url::loopback_origin(&parsed)?;
    let token = value["token"]
        .as_str()
        .ok_or("Missing DSH companion pairing token")?;
    if !(32..=256).contains(&token.len())
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("Invalid DSH companion pairing token".into());
    }
    if let Some(label) = value.get("profileLabel") {
        super::parser::text(label, 256)?;
    }
    let profile_id = match value.get("profileId") {
        None => None,
        Some(value) => {
            let id = value
                .as_str()
                .filter(|id| {
                    id.len() == 64
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
                .ok_or("Invalid DSH companion profile identity")?;
            Some(id.to_string())
        }
    };
    let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "Invalid DSH companion pairing header")?;
    bearer.set_sensitive(true);
    Ok(Record {
        instance_id: id.into(),
        profile_id,
        origin,
        bearer,
        path: PathBuf::new(),
        directory: PathBuf::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn record() -> Value {
        json!({"schemaVersion":1,"instanceId":"f5409f91-ec39-462d-9964-596e18e2a36c","origin":"http://127.0.0.1:12345","token":"SECRET_0123456789_0123456789_0123456789","profileLabel":"Desktop"})
    }
    #[test]
    fn profile_identity_is_optional_bounded_public_hash() {
        assert!(parse_record(&serde_json::to_vec(&record()).unwrap())
            .unwrap()
            .profile_id
            .is_none());
        let mut value = record();
        value["profileId"] = json!("a".repeat(64));
        assert_eq!(
            parse_record(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .profile_id,
            Some("a".repeat(64))
        );
        for invalid in [
            json!("SECRET/path"),
            json!("A".repeat(64)),
            json!("a".repeat(63)),
            json!(null),
            json!(7),
        ] {
            value["profileId"] = invalid;
            let error = parse_record(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap();
            assert_eq!(error, "Invalid DSH companion profile identity");
        }
    }
    #[test]
    fn accepts_only_dedicated_loopback_discovery_without_secret_errors() {
        let parsed = parse_record(&serde_json::to_vec(&record()).unwrap()).unwrap();
        assert_eq!(parsed.origin.as_str(), "http://127.0.0.1:12345/");
        assert!(parsed.bearer.is_sensitive());
        assert!(!format!("{:?}", parsed.bearer).contains("SECRET"));
        for origin in [
            "http://evil.org",
            "http://127.0.0.1:4/api",
            "http://u:SECRET@127.0.0.1:4",
            "http://127.0.0.1:4/?token=SECRET",
        ] {
            let mut value = record();
            value["origin"] = json!(origin);
            assert!(!parse_record(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap()
                .contains("SECRET"));
        }
        for field in ["schemaVersion", "instanceId", "token"] {
            let mut value = record();
            value[field] = json!("SECRET");
            assert!(!parse_record(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap()
                .contains("SECRET"));
        }
    }
}
