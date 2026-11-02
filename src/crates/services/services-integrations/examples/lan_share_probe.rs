//! Manual check of LAN sharing between two machines.
//!
//! Run on both machines of one local network:
//!
//! ```text
//! lan_share_probe [--identity <path>] [--seconds <n>]
//! ```
//!
//! Each side advertises itself, browses, and for every compatible device it
//! finds opens a channel pinned to the advertised key and makes one
//! encrypted round trip. The identity defaults to a file in the current
//! directory so the probe never touches the product's own identity.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openbitfun_services_integrations::lan_share::discovery::sharing_ipv4_addresses;
use openbitfun_services_integrations::lan_share::{
    connect, Advertisement, ChannelConfig, Discovery, DiscoveryEvent, LanListener, PeerIdentity,
    RequestHandler, ShareIdentity,
};
use serde_json::{json, Value};

struct Echo;

#[async_trait]
impl RequestHandler for Echo {
    async fn handle(
        &self,
        peer: &PeerIdentity,
        method: &str,
        body: Value,
    ) -> Result<Value, String> {
        println!(
            "request {method} from {} ({}, {})",
            peer.display_name, peer.device_name, peer.fingerprint
        );
        Ok(json!({ "echo": body }))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut identity_path = std::env::current_dir()?.join("lan-share-probe-identity.json");
    let mut seconds = 60u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--identity" => {
                identity_path = std::env::current_dir()?.join(args.next().expect("a path"))
            }
            "--seconds" => seconds = args.next().expect("a number").parse()?,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }

    let identity = Arc::new(ShareIdentity::load_or_create(&identity_path)?);
    println!(
        "this device: {} ({}) id {} fingerprint {}",
        identity.display_name(),
        identity.device_name(),
        identity.device_id(),
        identity.fingerprint()
    );
    let config = ChannelConfig::new(identity.clone());
    let addresses = sharing_ipv4_addresses(false);
    let listener = LanListener::bind(config.clone(), Arc::new(Echo), &addresses).await?;
    println!(
        "listening on {:?} port {}",
        listener.addresses(),
        listener.port()
    );

    let discovery = Discovery::new(identity.device_id(), false)?;
    discovery.advertise(&Advertisement {
        device_id: identity.device_id().to_string(),
        device_name: identity.device_name().to_string(),
        display_name: identity.display_name().to_string(),
        public_key: identity.public_key(),
        port: listener.port(),
    })?;
    let mut browser = discovery.browse()?;
    let started = Instant::now();
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => break,
            event = browser.next() => match event {
                Some(DiscoveryEvent::Found(peer)) => {
                    println!(
                        "found after {:.1}s: {} ({}) fingerprint {} protocol {} at {:?}:{}",
                        started.elapsed().as_secs_f32(),
                        peer.display_name,
                        peer.device_name,
                        peer.fingerprint,
                        peer.protocol_version,
                        peer.addresses,
                        peer.port
                    );
                    if !peer.is_compatible() {
                        println!("  needs an update; not connecting");
                        continue;
                    }
                    let address = SocketAddr::from((peer.addresses[0], peer.port));
                    let round_trip = Instant::now();
                    match connect(&config, address, Some(&peer.public_key), None).await {
                        Ok(connection) => match connection
                            .call("echo", json!(format!("hello from {}", identity.display_name())))
                            .await
                        {
                            Ok(answer) => println!(
                                "  encrypted round trip in {} ms: {answer}",
                                round_trip.elapsed().as_millis()
                            ),
                            Err(error) => println!("  call failed: {error}"),
                        },
                        Err(error) => println!("  connect failed: {error}"),
                    }
                }
                Some(DiscoveryEvent::Lost { device_id }) => {
                    println!("lost after {:.1}s: {device_id}", started.elapsed().as_secs_f32());
                }
                None => break,
            }
        }
    }
    discovery.stop_advertising()?;
    println!("stopped advertising");
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}
