mod tls;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use anyhow::Context;
use anyhow::Result;
use rcgen::generate_simple_self_signed;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

pub use tls::{connect, peer_id, secure_code, server_config, BoxedStream, TlsStream};
pub use tokio_rustls::TlsAcceptor;

#[derive(Clone)]
pub struct Identity {
    pub id: String,
    pub(crate) certificate: Vec<u8>,
    pub(crate) private_key: Arc<Vec<u8>>,
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let key = generate_simple_self_signed(vec!["file-trans.local".to_owned()])?;
        Ok(Self::from_parts(
            key.cert.der().to_vec(),
            key.signing_key.serialize_der(),
        ))
    }

    fn from_parts(certificate: Vec<u8>, private_key: Vec<u8>) -> Self {
        Self {
            id: fingerprint(&certificate),
            certificate,
            private_key: Arc::new(private_key),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    pub fn load_or_create(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory)?;
        let path = directory.join("device-cert.der");
        if path.exists() {
            let certificate = std::fs::read(&path).context("读取设备证书失败")?;
            let entry = keyring::Entry::new("File Trans", &fingerprint(&certificate))?;
            let private_key = entry
                .get_secret()
                .context("无法从系统凭据库读取设备私钥，请检查权限；不会自动更换身份")?;
            let identity = Self::from_parts(certificate, private_key);
            server_config(&identity).context("设备证书与私钥不匹配")?;
            return Ok(identity);
        }
        let identity = Self::generate()?;
        let entry = keyring::Entry::new("File Trans", &identity.id)?;
        entry
            .set_secret(&identity.private_key)
            .context("保存设备私钥到系统凭据库失败")?;
        if let Err(error) = std::fs::write(path, &identity.certificate) {
            let _ = entry.delete_credential();
            return Err(error.into());
        }
        Ok(identity)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    pub fn load_or_create(_directory: &Path) -> Result<Self> {
        anyhow::bail!("当前桌面版本仅支持 Windows、macOS 和 Linux 的系统凭据库")
    }
}

pub fn fingerprint(certificate: &[u8]) -> String {
    hex::encode(Sha256::digest(certificate))
}
