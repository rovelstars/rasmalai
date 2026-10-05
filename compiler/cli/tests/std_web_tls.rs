use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-tls-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_certs() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut ca_params = rcgen::CertificateParams::new(vec![]).unwrap();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Rnx Test CA");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_issuer = rcgen::Issuer::new(ca_params, ca_key);

    let mut leaf_params = rcgen::CertificateParams::new(vec![]).unwrap();
    leaf_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "localhost");
    leaf_params.subject_alt_names = vec![
        rcgen::SanType::DnsName("localhost".try_into().unwrap()),
        rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()),
    ];
    leaf_params.is_ca = rcgen::IsCa::ExplicitNoCa;
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf_cert = leaf_params.signed_by(&leaf_key, &ca_issuer).unwrap();
    (
        ca_cert.der().to_vec(),
        leaf_cert.der().to_vec(),
        leaf_key.serialize_der(),
    )
}

fn tls_config_for(leaf_der: Vec<u8>, key_der: Vec<u8>) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(rustls::ALL_VERSIONS)
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf_der)],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key_der).into(),
            )
            .unwrap(),
    )
}

fn serve_tls(listener: TcpListener, config: Arc<rustls::ServerConfig>, n: usize) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for _ in 0..n {
            let (sock, _) = listener.accept().unwrap();
            let mut sock = sock;
            sock.set_nonblocking(false).ok();
            let mut conn = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::Stream::new(&mut conn, &mut sock);
            let _ = serve_one(&mut tls);
        }
    })
}

fn serve_one(tls: &mut rustls::Stream<'_, rustls::ServerConnection, std::net::TcpStream>) -> std::io::Result<()> {
    let mut req = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let k = tls.read(&mut buf)?;
        if k == 0 {
            break;
        }
        req.extend_from_slice(&buf[..k]);
        if req.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if req.len() > 65536 {
            break;
        }
    }
    let body = if req.starts_with(b"GET /secure ") {
        &b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nsecure hello"[..]
    } else {
        &b"HTTP/1.1 404 Not Found\r\nContent-Length: 7\r\nConnection: close\r\n\r\nmissing"[..]
    };
    tls.write_all(body)?;
    Ok(())
}

struct TlsFixture {
    dir: PathBuf,
    port: u16,
    server: Option<std::thread::JoinHandle<()>>,
}

impl TlsFixture {
    fn new(tag: &str, conns: usize) -> TlsFixture {
        let dir = fresh_dir(tag);
        let (ca_der, leaf_der, key_der) = make_certs();
        std::fs::write(dir.join("ca.der"), &ca_der).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = serve_tls(listener, tls_config_for(leaf_der, key_der), conns);
        TlsFixture { dir, port, server: Some(server) }
    }

    fn ca_path(&self) -> PathBuf {
        self.dir.join("ca.der")
    }

    fn run_rnx(&self, src: &str, backend: Option<&str>) -> std::process::Output {
        let prog = self.dir.join("main.rnx");
        let src = src.replace("__PORT__", &self.port.to_string());
        std::fs::write(&prog, src).unwrap();
        let mut cmd = Command::new(rnx());
        cmd.arg("run").env("NO_COLOR", "1");
        cmd.env("RNX_TEST_TLS_CA_DER", self.ca_path());
        if let Some(b) = backend {
            cmd.arg("--backend").arg(b);
        }
        cmd.arg(&prog);
        cmd.output().unwrap()
    }

    fn finish(mut self) {
        if let Some(srv) = self.server.take() {
            srv.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const FETCH_SECURE: &str = r#"import { fetch, Response } from "@std/web";

let r: Response = await fetch("https://127.0.0.1:__PORT__/secure");
print(r.status);
print(r.text());
"#;

#[test]
fn fetch_https_round_trip_against_local_ca() {
    for backend in [Some("interpreter"), Some("cranelift"), Some("llvm")] {
        let tag = format!("ok-{}", backend.unwrap_or("interp"));
        let fix = TlsFixture::new(&tag, 1);
        let out = fix.run_rnx(FETCH_SECURE, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{tag} failed: {stderr}");
        assert_eq!(stdout, "200\nsecure hello\n", "{tag} stdout mismatch");
        fix.finish();
    }
}

const BAD_DOMAIN: &str = r#"import { TcpStream, TlsStream } from "@std/net";

let raw: TcpStream = await TcpStream.connect("127.0.0.1", __PORT__);
let tls: TlsStream = await TlsStream.connect(raw, "wrong.invalid");
print("unreachable");
"#;

#[test]
fn tls_rejects_wrong_domain() {
    let fix = TlsFixture::new("baddomain", 1);
    let out = fix.run_rnx(BAD_DOMAIN, None);
    assert!(!out.status.success(), "wrong domain must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(combined.contains("handshake failed"), "{combined}");
    fix.finish();
}

const FETCH_EXAMPLE: &str = r#"import { fetch, Response } from "@std/web";

let r: Response = await fetch("https://example.com/");
print(r.status);
print(r.text().length() > 0);
"#;

#[test]
#[ignore]
fn fetch_https_online_example_com() {
    let dir = fresh_dir("online");
    let prog = dir.join("main.rnx");
    std::fs::write(&prog, FETCH_EXAMPLE).unwrap();
    let out = Command::new(rnx())
        .arg("run")
        .env("NO_COLOR", "1")
        .arg(&prog)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "online fetch failed: {stderr}");
    assert_eq!(stdout, "200\ntrue\n", "online stdout mismatch: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
