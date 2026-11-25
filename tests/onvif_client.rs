use m7s_rs::onvif::{OnvifClient, discover_at, probe_message, run};
use tokio::net::{TcpListener, UdpSocket};

#[test]
fn onvif_client_against_mock() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(run(listener));
            let client = OnvifClient::new(&format!("http://127.0.0.1:{port}/onvif/device_service"));

            let info = client.get_device_information().await.unwrap();
            assert!(info.contains("m7s-rs"), "{info}");
            let profiles = client.get_profiles().await.unwrap();
            assert!(profiles.contains("profile1"), "{profiles}");
            let uri = client.get_stream_uri("profile1").await.unwrap();
            assert!(uri.starts_with("rtsp://"), "{uri}");
            let moved = client.continuous_move(0.5, -0.5, 0.0).await.unwrap();
            assert!(moved.contains("ContinuousMoveResponse"), "{moved}");
            let authed = OnvifClient::new(&format!("http://127.0.0.1:{port}/onvif/device_service")).with_auth("admin", "pw");
            assert!(authed.get_device_information().await.unwrap().contains("m7s-rs"));
            let stopped = client.ptz_stop().await.unwrap();
            assert!(stopped.contains("StopResponse"), "{stopped}");
        });
}

#[test]
fn onvif_discovery_client() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let addr = socket.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let (n, peer) = socket.recv_from(&mut buf).await.unwrap();
                assert!(String::from_utf8_lossy(&buf[..n]).contains("Probe"));
                let reply = "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" xmlns:d=\"http://schemas.xmlsoap.org/ws/2005/04/discovery\"><s:Body><d:ProbeMatches><d:ProbeMatch><d:XAddrs>http://127.0.0.1:8083/onvif/device_service</d:XAddrs><d:Types>dn:NetworkVideoTransmitter</d:Types><d:Scopes>onvif://www.onvif.org/name/m7s</d:Scopes></d:ProbeMatch></d:ProbeMatches></s:Body></s:Envelope>";
                socket.send_to(reply.as_bytes(), peer).await.unwrap();
            });
            let found = discover_at(addr, std::time::Duration::from_secs(1)).await.unwrap();
            assert_eq!(found.len(), 1);
            assert!(found[0].endpoint.contains("device_service"), "{}", found[0].endpoint);
            assert!(found[0].types.contains("NetworkVideoTransmitter"));
            assert!(!probe_message().is_empty());
            server.await.unwrap();
        });
}
