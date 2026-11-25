use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run())
}

async fn run() -> Result<()> {
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let api = APIBuilder::new().with_media_engine(media).build();
    let a = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);
    let b = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    let (tx, mut rx) = mpsc::channel::<Arc<RTCDataChannel>>(1);
    b.on_data_channel(Box::new(move |dc| {
        let tx = tx.clone();
        Box::pin(async move {
            let _ = tx.send(dc).await;
        })
    }));

    let dc = a
        .create_data_channel("test", Some(RTCDataChannelInit::default()))
        .await?;
    dc.on_open(Box::new(|| {
        println!("DC_OPEN");
        Box::pin(async {})
    }));

    let offer = a.create_offer(None).await?;
    a.set_local_description(offer).await?;
    let mut ga = a.gathering_complete_promise().await;
    let _ = ga.recv().await;
    let offer = a.local_description().await.unwrap();
    b.set_remote_description(RTCSessionDescription::offer(offer.sdp)?)
        .await?;
    let answer = b.create_answer(None).await?;
    b.set_local_description(answer).await?;
    let mut gb = b.gathering_complete_promise().await;
    let _ = gb.recv().await;
    let answer = b.local_description().await.unwrap();
    a.set_remote_description(RTCSessionDescription::answer(answer.sdp)?)
        .await?;

    tokio::time::sleep(Duration::from_secs(5)).await;
    println!(
        "a_ice={:?} b_ice={:?} dc={:?}",
        a.ice_connection_state(),
        b.ice_connection_state(),
        dc.ready_state()
    );
    let received = rx.try_recv();
    println!("data_channel_received={}", received.is_ok());
    let _ = a.close().await;
    let _ = b.close().await;
    Ok(())
}
