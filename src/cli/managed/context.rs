use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const VERSION: u32 = 1;
const MAX_CONTEXT_BYTES: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    version: u32,
    owner: String,
    secret: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    version: u32,
    secret_hash: String,
}

pub fn initialize(path: &Path) -> Result<String> {
    let credential = Credential {
        version: VERSION,
        owner: Uuid::new_v4().to_string(),
        secret: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
    };
    let directory = crate::session::get_app_dir()?.join("managers");
    fs::create_dir_all(&directory)?;
    let registration = Registration {
        version: VERSION,
        secret_hash: hash(&credential.secret),
    };
    let registration_path = directory.join(format!("{}.json", credential.owner));
    write_new(&registration_path, &registration)?;
    if let Err(error) = write_new(path, &credential) {
        fs::remove_file(&registration_path).context("Rolling back manager registration")?;
        return Err(error);
    }
    Ok(credential.owner)
}

pub fn authenticate(path: &Path) -> Result<String> {
    let credential: Credential = read(path).context("Reading manager context")?;
    ensure!(
        credential.version == VERSION,
        "Unsupported manager context version"
    );
    ensure!(
        Uuid::parse_str(&credential.owner)?.to_string() == credential.owner,
        "Invalid manager identity"
    );
    ensure!(
        credential.secret.len() == 64
            && credential
                .secret
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "Invalid manager credential"
    );
    let registration: Registration = read(
        &crate::session::get_app_dir()?
            .join("managers")
            .join(format!("{}.json", credential.owner)),
    )
    .context("Manager identity is not registered")?;
    ensure!(
        registration.version == VERSION && registration.secret_hash == hash(&credential.secret),
        "Invalid manager credential"
    );
    Ok(credential.owner)
}

fn hash(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_CONTEXT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CONTEXT_BYTES,
        "Manager context exceeds size limit"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("Creating {}", path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn context_is_stable_and_forged_owner_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(temp.path());
        let first = temp.path().join("first.json");
        let second = temp.path().join("second.json");
        let owner = initialize(&first).unwrap();
        assert_eq!(authenticate(&first).unwrap(), owner);
        assert!(initialize(&first).is_err());
        let other_owner = initialize(&second).unwrap();
        let mut forged: Credential = read(&first).unwrap();
        forged.owner = other_owner;
        fs::write(&first, serde_json::to_vec(&forged).unwrap()).unwrap();
        assert!(authenticate(&first).is_err());
    }
}
