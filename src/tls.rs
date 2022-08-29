use rustls::{Certificate, PrivateKey, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncReadExt;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),
    #[error("The provided private key file contains no PKCS8 private key")]
    NoPrivateKey,
    #[error("The provided certificate file containers no certificate")]
    NoCertificates,
}

pub async fn configure_tls<P: AsRef<Path>, P1: AsRef<Path>>(
    cert_path: P,
    privkey_path: P1,
) -> Result<ServerConfig, TlsError> {
    let certificate_pem_bytes = read_file_to_vec(cert_path).await?;
    let privkey_pem_bytes = read_file_to_vec(privkey_path).await?;

    // Extract the certificates
    let mut cursor = Cursor::new(certificate_pem_bytes);
    let raw_certificates = certs(&mut cursor)?;
    let certificates = raw_certificates
        .into_iter()
        .map(|x| Certificate(x))
        .collect::<Vec<_>>();

    if certificates.is_empty() {
        return Err(TlsError::NoCertificates);
    }

    // Extract the private keys
    let mut cursor = Cursor::new(privkey_pem_bytes);
    let raw_privkeys = pkcs8_private_keys(&mut cursor)?;
    let mut privkeys = raw_privkeys
        .into_iter()
        .map(|x| PrivateKey(x))
        .collect::<Vec<_>>();

    if privkeys.is_empty() {
        return Err(TlsError::NoPrivateKey);
    }

    let privkey = privkeys.remove(0);

    let config = ServerConfig::builder()
        .with_safe_default_cipher_suites()
        .with_safe_default_kx_groups()
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certificates, privkey)
        .unwrap();

    Ok(config)
}

async fn read_file_to_vec<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, TlsError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(TlsError::FileNotFound(path.to_path_buf()));
    }

    let mut f = fs::File::open(path).await?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).await?;

    Ok(buf)
}
