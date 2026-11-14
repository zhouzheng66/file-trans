use crate::{fingerprint, Identity};
use anyhow::{anyhow, ensure, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring::default_provider, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    DigitallySignedStruct, DistinguishedName, Error, SignatureScheme,
};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;

pub trait Transport: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Transport for T {}
pub type BoxedStream = Box<dyn Transport>;
pub type TlsStream = tokio_rustls::TlsStream<BoxedStream>;

// 自签名证书引导：保留 TLS 的签名验证；业务层另行执行配对授权。
#[derive(Debug)]
struct DeviceVerifier {
    expected_id: Option<String>,
}

impl ServerCertVerifier for DeviceVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        if self
            .expected_id
            .as_ref()
            .is_some_and(|id| *id != fingerprint(end_entity.as_ref()))
        {
            return Err(Error::General("设备证书指纹不匹配".into()));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl ClientCertVerifier for DeviceVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _certificate: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        ServerCertVerifier::verify_tls12_signature(self, message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        ServerCertVerifier::verify_tls13_signature(self, message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ServerCertVerifier::supported_verify_schemes(self)
    }
}

fn certificate(identity: &Identity) -> Vec<CertificateDer<'static>> {
    vec![CertificateDer::from(identity.certificate.clone())]
}

fn private_key(identity: &Identity) -> PrivateKeyDer<'static> {
    PrivatePkcs8KeyDer::from(identity.private_key.as_ref().clone()).into()
}

pub fn server_config(identity: &Identity) -> Result<Arc<rustls::ServerConfig>> {
    Ok(Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(DeviceVerifier { expected_id: None }))
            .with_single_cert(certificate(identity), private_key(identity))?,
    ))
}

pub async fn connect<S: Transport + 'static>(
    stream: S,
    identity: &Identity,
    expected_id: Option<&str>,
) -> Result<TlsStream> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(DeviceVerifier {
            expected_id: expected_id.map(str::to_owned),
        }))
        .with_client_auth_cert(certificate(identity), private_key(identity))?;
    let stream = TlsConnector::from(Arc::new(config))
        .connect(
            ServerName::try_from("file-trans.local")?,
            Box::new(stream) as BoxedStream,
        )
        .await?;
    Ok(TlsStream::Client(stream))
}

pub fn peer_id(stream: &TlsStream) -> Result<String> {
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| anyhow!("对端未提供设备证书"))?;
    Ok(fingerprint(certificate.as_ref()))
}

// TLS exporter 绑定本次握手；中间人建立的两条独立 TLS 会话不会产生相同校验码。
pub fn secure_code(stream: &TlsStream) -> Result<String> {
    let mut bytes = [0_u8; 8];
    match stream {
        TlsStream::Client(stream) => {
            stream.get_ref().1.export_keying_material(
                &mut bytes,
                b"file-trans pairing v1",
                None,
            )?;
        }
        TlsStream::Server(stream) => {
            stream.get_ref().1.export_keying_material(
                &mut bytes,
                b"file-trans pairing v1",
                None,
            )?;
        }
    }
    let value = u64::from_be_bytes(bytes) % 100_000_000;
    let code = format!("{value:08}");
    ensure!(code.len() == 8, "安全校验码生成失败");
    Ok(format!("{} {}", &code[..4], &code[4..]))
}
