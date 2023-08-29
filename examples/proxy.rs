use anyhow::{Ok, Result as AResult};
use nix::sched::CloneFlags;
use std::{convert::TryInto, env, error::Error, path::PathBuf};
use tokio::sync::oneshot;

use netlink_proto::{
    new_connection_from_socket, new_connection_with_socket, Connection,
    NetlinkCodec,
};
use netlink_sys::{
    protocols::NETLINK_ROUTE,
    proxy::{self, NProxyID, NetlinkProxy, ProxyParam},
};

use futures::StreamExt;
use netlink_packet_core::{
    NetlinkHeader, NetlinkMessage, NLM_F_DUMP, NLM_F_REQUEST,
};
use netlink_packet_route::{LinkMessage, RtnlMessage};
use netlink_proto::{new_connection, sys::SocketAddr};
use tokio::process::Command;

#[tokio::main]
async fn main() -> AResult<()> {
    let p: PathBuf = "./p.sock".parse()?;
    let args: Vec<String> = env::args().collect();
    let mut parsed: WhoIAM = WhoIAM::Hub;
    env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .init();
    if args.len() == 2 {
        let b: u8 = args[1].parse()?;
        parsed = unsafe { std::mem::transmute(b) };
    }
    match parsed {
        WhoIAM::Hub => {
            let (sx, tx) = oneshot::channel();
            let ctx = NetlinkProxy::new(p)?;
            let mut g = ctx.pending.write().await;
            g.insert(
                NProxyID(0),
                ProxyParam {
                    cb: sx,
                    proto: NETLINK_ROUTE,
                },
            );
            drop(g);
            tokio::spawn(ctx.serve());

            let x: u8 = unsafe { std::mem::transmute(WhoIAM::Proxy) };
            let mut cmd = Command::new(std::env::current_exe()?);
            cmd.arg(x.to_string());
            let _h = cmd.spawn()?;

            let ts = tx.await?;
            let (conn, mut handle, _) =
                new_connection_from_socket::<_, _, NetlinkCodec>(ts);
            tokio::spawn(conn);
            let mut nl_hdr = NetlinkHeader::default();
            nl_hdr.flags = NLM_F_DUMP | NLM_F_REQUEST;
            let request = NetlinkMessage::new(
                nl_hdr,
                RtnlMessage::GetLink(LinkMessage::default()).into(),
            );

            // Send the request
            let mut response =
                handle.request(request, SocketAddr::new(0, 0))?;

            // Print all the messages received in response
            while let Some(packet) = response.next().await {
                println!("<<< {packet:?}");
            }
        }
        WhoIAM::Proxy => {
            nix::sched::unshare(CloneFlags::CLONE_NEWNET)?;
            proxy::proxy(p, Some(NProxyID(0))).await?;
            // should exit briefly
            println!("proxy process exits");
        }
    }
    Ok(())
}

enum WhoIAM {
    Hub = 0,
    Proxy,
}
