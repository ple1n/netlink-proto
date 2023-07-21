use anyhow::{Ok, Result as AResult};
use std::{convert::TryInto, env, error::Error, path::PathBuf};
use tokio::sync::oneshot;

use netlink_proto::new_connection_with_socket;
use netlink_sys::{
    protocols::NETLINK_ROUTE,
    proxy::{self, ProxyCtx, ProxyCtxP, ProxySocket, ProxySocketType},
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
        .filter_level(log::LevelFilter::Info)
        .init();
    if args.len() == 2 {
        let b: u8 = args[1].parse()?;
        parsed = unsafe { std::mem::transmute(b) };
    }
    match parsed {
        WhoIAM::Hub => {
            let (sx, rx) = oneshot::channel::<_>();
            tokio::spawn(async {
                // put all this in an async block, because it has to be 'static.
                let t1 = async {
                    let mut ctx = ProxyCtx::new(p)?;
                    ctx.get_subs(1).await?;
                    let mut params = ProxyCtxP {
                        shared: &mut ctx,
                        inode: proxy::get_inode_self_ns()?,
                    };

                    let (mut conn, handle, m) =
                        new_connection_with_socket::<
                            _,
                            ProxySocket<{ ProxySocketType::PollRecvFrom }>,
                        >(NETLINK_ROUTE, &mut params)?;
                    conn.socket_mut().init().await;
                    sx.send((handle, m)).unwrap();
                    conn.await;

                    Ok(())
                };
                let t2 = async {
                    let x: u8 = unsafe { std::mem::transmute(WhoIAM::Proxy) };
                    let mut cmd = Command::new(std::env::current_exe()?);
                    cmd.arg(x.to_string());
                    let h = cmd.spawn()?;
                    Ok(())
                };
                // XXX: t1 must be polled before t2 I guess
                let r = tokio::try_join!(t1, t2)?;

                Ok(())
            });

            let (mut handle, _) = rx.await?;

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
            proxy::proxy::<{ ProxySocketType::PollRecvFrom }>(p).await?;
        }
    }
    Ok(())
}

enum WhoIAM {
    Hub = 0,
    Proxy,
}
