use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::task;
use log::{debug, error, info};
use std::{fs};
use std::sync::Arc;
use std::option::Option::Some;
use std::collections::{BTreeMap};
use std::path::Path;
use std::time::{Duration, Instant};
use async_timer::timed;
use async_std::io::ErrorKind;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = async_channel::Sender<T>;
type Receiver<T> = async_channel::Receiver<T>;

struct Request {
    connection_id: u32,
}

enum Event {
    Connection {
        stream: Arc<TcpStream>,
    },
    ReadyResponse {
        connection_id: u32,
        payload: Arc<Vec<u8>>,
    },
    TimerCheckAliveConnection,
    CloseConnection {
        stream: Arc<TcpStream>,
        connection_id: u32,
    },
    CompletedResponse {
        connection_id: u32,
    },
}

fn spawn_write<F>(fut: F, tx_ev: Sender<Event>, key: u32) -> task::JoinHandle<(Result<()>)>
    where
        F: Future<Output = Result<()>> + Send + 'static,
{
    task::spawn(async move {
        match fut.await {
            Ok(_) => {
                tx_ev.send(Event::CompletedResponse {connection_id: key}).await?;
            },
            Err(err) => {
                error!("{}", err.to_string());
            }
        }
        Ok(())
    })
}

fn spawn_read<F>(fut: F, tx_ev: Sender<Event>, key: u32, stream: Arc<TcpStream>) -> task::JoinHandle<(Result<()>)>
    where
        F: Future<Output = Result<(usize)>> + Send + 'static,
{
    task::spawn(async move {
        match fut.await {
            Ok(value) => {
                if value == 0 {
                    tx_ev.send(Event::CloseConnection {stream, connection_id: key}).await?;
                }
            }
            Err(err) => {
                error!("{}", err.to_string());
                tx_ev.send(Event::CloseConnection {stream, connection_id: key}).await?;
            }
        }
        Ok(())
    })
}

async fn broker_loop(rx_req: Sender<Request>, mut tx_ev: Sender<Event>, mut rx_ev: Receiver<Event>) -> Result<()> {
    let mut idx = 0;
    let mut connections: BTreeMap<u32, Arc<TcpStream>> = BTreeMap::new();
    while let Some(event) = rx_ev.next().await {
        match event {
            Event::Connection {stream} => {
                debug!("Number of connected clients = {}", connections.len());
                connections.insert(idx, stream.clone());
                if let Err(err) = rx_req.send(Request{ connection_id: idx }).await {
                    panic!(err.to_string());
                }
                idx = idx.wrapping_add(1);
            },
            Event::ReadyResponse { connection_id, payload } => {
                if let Some(stream) = connections.get_mut(&connection_id) {
                    spawn_write(write(stream.clone(), payload.clone()), tx_ev.clone(), connection_id);
                }
            },
            Event::TimerCheckAliveConnection{} => {
                let mut keys = vec![];
                for (key, stream) in &connections {
                    spawn_read(read(stream.clone()), tx_ev.clone(), *key, stream.clone());
                }
                for key in keys {
                    connections.remove(&key);
                }
            },
            Event::CloseConnection { stream, connection_id } => {
                debug!("Connection {} closed by client", stream.peer_addr()?);
                connections.remove(&connection_id);
            },
            Event::CompletedResponse {connection_id} => {
                if let Some(stream) = connections.get_mut(&connection_id) {
                    debug!("Response completed. Closed {} connection by server", stream.peer_addr()?);
                    stream.shutdown(std::net::Shutdown::Both);
                    connections.remove(&connection_id);
                }
            }
        }
    }
    Ok(())
}

async fn write(stream: Arc<TcpStream>, payload: Arc<Vec<u8>>) -> Result<()> {
    let mut stream = &*stream;
    let header_image = format!("HTTP/1.1 200 OK\x0d\x0aConnection: keep-alive\x0d\x0aMax-Age: 0\x0d\x0aExpires: 0\x0d\x0aCache-Control: no-cache, private\x0d\x0aPragma: no-cache\x0d\x0aContent-Type: text/plan\x0d\x0aContent-Length: {}\x0d\x0a\x0d\x0a", payload.len());
    stream.write_all(header_image.as_bytes()).await?;
    stream.write_all(&payload[..]).await?;
    Ok(())
}

async fn read(stream: Arc<TcpStream>) -> Result<(usize)> {
    let mut stream = &*stream;
    let mut buf: [u8;1024] = [0; 1024];
    Ok(stream.read(&mut buf).await?)
}

async fn timer_check_alive_loop(tx_ev: Sender<Event>) -> Result<()> {
    let mut interval = async_timer::Interval::platform_new(core::time::Duration::from_secs(30));
    loop {
        tx_ev.send(Event::TimerCheckAliveConnection{}).await?;
        interval.as_mut().await;
    }
    Ok(())
}

async fn executor(mut rx_req: Receiver<Request>, tx_ev: Sender<Event>) -> Result<()> {
    while let Some(req) = rx_req.next().await {
        let content = String::from("Hello world!").as_bytes().iter().map(|x|*x).collect::<Vec<u8>>();
        tx_ev.send(Event::ReadyResponse { connection_id: req.connection_id, payload: Arc::new(content)}).await?;
    }
    Ok(())
}

pub fn run(address: &str) -> Result<()> {
    let (tx_ev, rx_ev): (Sender<Event>, Receiver<Event>) = async_channel::unbounded();
    let (tx_req, rx_req): (Sender<Request>, Receiver<Request>) = async_channel::unbounded();
    task::block_on(async move {
        let handle_executor = task::spawn(executor(rx_req, tx_ev.clone()));
        let handle_broker = task::spawn(broker_loop(tx_req, tx_ev.clone(), rx_ev));
        let handle_timer = task::spawn(timer_check_alive_loop(tx_ev.clone()));
        let listener = TcpListener::bind(address).await?;
        info!("Listening on {}", address);
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            debug!("Accepted from: {}", stream.peer_addr()?);
            tx_ev.send(Event::Connection {stream: Arc::new(stream)}).await?;
        }
        handle_broker.await?;
        handle_executor.await?;
        handle_timer.await?;
        Ok(())
    })
}