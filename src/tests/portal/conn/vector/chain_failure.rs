use super::*;

#[tokio::test]
async fn native_portal_chain_preserves_upstream_dial_failure() {
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_address = unavailable.local_addr().unwrap();
    drop(unavailable);
    let runtime = start_chain_runtime("udp", "udp").await;

    timeout(TEST_TIMEOUT, async {
        let mut socks = TcpStream::connect(runtime.socks).await.unwrap();
        negotiate_socks(&mut socks).await;
        socks
            .write_all(&ip_request(1, unavailable_address))
            .await
            .unwrap();
        assert_eq!(read_ipv4_reply_code(&mut socks).await, 4);
    })
    .await
    .unwrap();

    runtime.stop().await;
}
