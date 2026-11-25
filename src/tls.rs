//! TLS server support (built-in HTTPS) and a self-signed test certificate.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;

pub fn load_server(cert_path: &Path, key_path: &Path) -> Result<TlsAcceptor> {
    let certs = pem_blocks(
        &std::fs::read_to_string(cert_path)
            .with_context(|| format!("read cert {}", cert_path.display()))?,
        "CERTIFICATE",
    )?
    .into_iter()
    .map(CertificateDer::from)
    .collect::<Vec<_>>();
    if certs.is_empty() {
        bail!("no CERTIFICATE block in {}", cert_path.display());
    }
    let key = load_key(key_path)?;
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn load_server_with_client_auth(
    cert_path: &Path,
    key_path: &Path,
    ca_path: &Path,
) -> Result<TlsAcceptor> {
    let certs = pem_blocks(
        &std::fs::read_to_string(cert_path)
            .with_context(|| format!("read cert {}", cert_path.display()))?,
        "CERTIFICATE",
    )?
    .into_iter()
    .map(CertificateDer::from)
    .collect::<Vec<_>>();
    let mut roots = rustls::RootCertStore::empty();
    for ca in pem_blocks(
        &std::fs::read_to_string(ca_path)
            .with_context(|| format!("read client CA {}", ca_path.display()))?,
        "CERTIFICATE",
    )? {
        roots.add(CertificateDer::from(ca))?;
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots)).build()?;
    let key = load_key(key_path)?;
    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn load_client(
    cert_path: &Path,
    key_path: &Path,
    ca_path: &Path,
) -> Result<rustls::ClientConfig> {
    let certs = pem_blocks(
        &std::fs::read_to_string(cert_path)
            .with_context(|| format!("read client cert {}", cert_path.display()))?,
        "CERTIFICATE",
    )?
    .into_iter()
    .map(CertificateDer::from)
    .collect::<Vec<_>>();
    let mut roots = rustls::RootCertStore::empty();
    for ca in pem_blocks(
        &std::fs::read_to_string(ca_path)
            .with_context(|| format!("read CA {}", ca_path.display()))?,
        "CERTIFICATE",
    )? {
        roots.add(CertificateDer::from(ca))?;
    }
    let key = load_key(key_path)?;
    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(certs, key)?)
}
/// Generate a self-signed ECDSA P-256 certificate for localhost (tests/dev).
pub fn self_signed() -> Result<(TlsAcceptor, CertificateDer<'static>, Vec<u8>)> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
        .map_err(|_| anyhow::anyhow!("generate ECDSA key"))?;
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
        .map_err(|_| anyhow::anyhow!("parse ECDSA key"))?;
    let public = key_pair.public_key().as_ref();

    let ec_public = oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]);
    let prime256v1 = oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]);
    let ecdsa_sha256 = oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]);
    let spki = seq(&[seq(&[ec_public.clone(), prime256v1]), bit_string(public)]);
    let alg = seq(std::slice::from_ref(&ecdsa_sha256));
    let name = seq(&[set(&[seq(&[oid(&[0x55, 0x04, 0x03]), utf8("localhost")])])]);
    let validity = seq(&[utc_time("240101000000Z"), utc_time("340101000000Z")]);
    let san = seq(&[
        oid(&[0x55, 0x1d, 0x11]),
        octet_string(&seq(&[dns_name("localhost")])),
    ]);
    let extensions = explicit(3, &seq(&[san]));
    let tbs = seq(&[
        explicit(0, &integer(2)),
        integer(1),
        alg.clone(),
        name.clone(),
        validity,
        name,
        spki,
        extensions,
    ]);
    let sig = key_pair
        .sign(&rng, &tbs)
        .map_err(|_| anyhow::anyhow!("sign certificate"))?;
    let cert = seq(&[tbs, alg, bit_string(sig.as_ref())]);
    let cert_der = CertificateDer::from(cert);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_ref().to_vec()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key)?;
    Ok((
        TlsAcceptor::from(Arc::new(config)),
        cert_der,
        pkcs8.as_ref().to_vec(),
    ))
}

/// Write a self-signed localhost cert/key pair as PEM (development only).
pub fn write_self_signed(cert_path: &Path, key_path: &Path) -> Result<()> {
    let (_acceptor, cert, key) = self_signed()?;
    std::fs::write(cert_path, pem_encode("CERTIFICATE", cert.as_ref()))?;
    std::fs::write(key_path, pem_encode("PRIVATE KEY", &key))?;
    Ok(())
}

fn pem_encode(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in body.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn load_key(key_path: &Path) -> Result<PrivateKeyDer<'static>> {
    let text = std::fs::read_to_string(key_path)
        .with_context(|| format!("read key {}", key_path.display()))?;
    if let Some(bytes) = pem_blocks(&text, "PRIVATE KEY")?.into_iter().next() {
        return Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(bytes)));
    }
    if let Some(bytes) = pem_blocks(&text, "RSA PRIVATE KEY")?.into_iter().next() {
        return Ok(PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(bytes)));
    }
    bail!("no private key block")
}
fn pem_blocks(text: &str, label: &str) -> Result<Vec<Vec<u8>>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut out = Vec::new();
    let mut in_block = false;
    let mut body = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line == begin {
            in_block = true;
            body.clear();
        } else if line == end {
            if !in_block {
                bail!("unexpected PEM end");
            }
            let bytes = STANDARD.decode(body.as_bytes()).context("decode PEM")?;
            out.push(bytes);
            in_block = false;
        } else if in_block {
            body.push_str(line);
        }
    }
    if in_block {
        bail!("unterminated PEM block");
    }
    Ok(out)
}

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 128 {
        out.push(len as u8);
    } else if len < 256 {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    der(0x30, &parts.concat())
}

fn set(parts: &[Vec<u8>]) -> Vec<u8> {
    der(0x31, &parts.concat())
}

fn oid(bytes: &[u8]) -> Vec<u8> {
    der(0x06, bytes)
}

fn integer(value: u8) -> Vec<u8> {
    der(0x02, &[value])
}

fn utf8(value: &str) -> Vec<u8> {
    der(0x0c, value.as_bytes())
}

#[allow(dead_code)]
fn boolean(value: bool) -> Vec<u8> {
    der(0x01, &[if value { 0xff } else { 0 }])
}

fn octet_string(bytes: &[u8]) -> Vec<u8> {
    der(0x04, bytes)
}

fn bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut content = vec![0];
    content.extend_from_slice(bytes);
    der(0x03, &content)
}

fn utc_time(value: &str) -> Vec<u8> {
    der(0x17, value.as_bytes())
}

fn explicit(tag: u8, content: &[u8]) -> Vec<u8> {
    der(0xa0 | tag, content)
}

fn dns_name(value: &str) -> Vec<u8> {
    der(0x82, value.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_builds() {
        let (_acceptor, cert, _key) = self_signed().unwrap();
        assert!(cert.as_ref().starts_with(&[0x30]));
    }
}
