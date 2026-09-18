use super::*;
use crate::common::{LogLevel, Logger};
use crate::portal::Portal;
use crate::portal::pairing::{BoxReader, BoxWriter};
use crate::transport::{MorphKeys, MorphTcpStream};
use std::time::Duration;
use tokio::io::AsyncReadExt;

async fn tls_response_tail(morph: bool, upstream: bool) {
    timeout(Duration::from_secs(10), async {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.cert.der().clone()).unwrap();
        let server = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der())
                    .into(),
            )
            .unwrap();
        let client = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        // A bounded carrier forces TLS to retain ciphertext under backpressure.
        let (client_io, server_io) = tokio::io::duplex(4096);
        let keys = morph.then(|| MorphKeys::derive(b"secret"));
        let client_io = MorphTcpStream::client(client_io, keys.clone()).unwrap();
        let server_io = MorphTcpStream::server(server_io, keys);
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let (client, server) = tokio::join!(
            connector.connect("localhost".try_into().unwrap(), client_io),
            acceptor.accept(server_io),
        );
        let mut client = client.unwrap();
        let (_server_read, server_write) = tokio::io::split(server.unwrap());
        let portal = Portal::new(
            url::Url::parse("portal://secret@127.0.0.1:2000").unwrap(),
            Logger::new(LogLevel::None, false),
        )
        .unwrap();
        let access = portal
            .inner
            .telemetry
            .start_access(|| panic!("inactive telemetry"));
        let (mut source, source_read) = tokio::io::duplex(32768);
        let (_idle_peer, idle_read) = tokio::io::duplex(1024);
        let payload: Vec<u8> = (0..128 * 1024).map(|i| (i % 251) as u8).collect();
        let producer = async {
            source.write_all(&payload).await.unwrap();
            // As with HTTP keep-alive, no EOF or next request flushes the tail.
            std::future::pending::<()>().await;
        };
        let (mut client_read, mut client_write, target_read, target_write): (
            BoxReader,
            BoxWriter,
            BoxReader,
            BoxWriter,
        ) = if upstream {
            (
                Box::pin(source_read),
                Box::pin(tokio::io::sink()),
                Box::pin(idle_read),
                Box::pin(server_write),
            )
        } else {
            (
                Box::pin(idle_read),
                Box::pin(server_write),
                Box::pin(source_read),
                Box::pin(tokio::io::sink()),
            )
        };
        let relay = relay_stream(
            portal.inner.clone(),
            &mut client_read,
            &mut client_write,
            (target_read, target_write),
            Some((Carrier::Quic, Carrier::TlsTcp)),
            &access,
        );
        let receive = async {
            let mut received = vec![0; payload.len()];
            client.read_exact(&mut received).await.unwrap();
            assert_eq!(received, payload);
        };
        tokio::select! {
            result = relay => panic!("relay ended before the response arrived: {result:?}"),
            () = producer => unreachable!(),
            () = receive => {},
        }
    })
    .await
    .expect("TLS response tail must arrive while both directions remain open");
}

#[tokio::test]
async fn tls_downlink_flushes_response_tail() {
    tls_response_tail(false, false).await;
}

#[tokio::test]
async fn morph_tls_downlink_flushes_response_tail() {
    tls_response_tail(true, false).await;
}

#[tokio::test]
async fn chained_tls_uplink_flushes_response_tail() {
    tls_response_tail(false, true).await;
}

#[tokio::test]
async fn chained_morph_tls_uplink_flushes_response_tail() {
    tls_response_tail(true, true).await;
}
