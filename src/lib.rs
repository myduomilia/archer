use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::{task, io};
use async_listen::{ListenExt, error_hint};
use log::{debug, error, info};
use std::sync::Arc;
use std::option::Option::Some;
use std::collections::{HashMap};
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = async_channel::Sender<T>;
type Receiver<T> = async_channel::Receiver<T>;

struct Request {
    connection_id: u32,
}

enum Event {
    OpenConnection {
        stream: Arc<TcpStream>,
    },
    SendingData {
        connection_id: u32,
        payload: Vec<u8>,
    },
    PackageReceived {
        connection_id: u32,
        payload: Vec<u8>,
    },
    TimerCheckAliveConnection,
    CloseConnection {
        stream: Arc<TcpStream>,
        connection_id: u32,
    },
    Disconnect {
        connection_id: u32,
    },
}

fn log_accept_error(e: &io::Error) {
    error!("Error: {}. Listener paused for 0.5s. {}", e, error_hint(e))
}

fn spawn_write<F>(fut: F, tx_ev: Sender<Event>, key: u32) -> task::JoinHandle<Result<()>>
    where
        F: Future<Output = Result<()>> + Send + 'static,
{
    task::spawn(async move {
        match fut.await {
            Ok(_) => {
                tx_ev.send(Event::Disconnect {connection_id: key}).await?;
            },
            Err(err) => {
                error!("Operation write error. {}", err.to_string());
            }
        }
        Ok(())
    })
}

fn spawn_read<F>(fut: F, tx_ev: Sender<Event>, key: u32, stream: Arc<TcpStream>) -> task::JoinHandle<Result<()>>
    where
        F: Future<Output = Result<usize>> + Send + 'static,
{
    task::spawn(async move {
        match fut.await {
            Ok(value) => {
                if value == 0 {
                    tx_ev.send(Event::CloseConnection {stream, connection_id: key}).await?;
                }
            }
            Err(err) => {
                error!("Operation read error. {}", err.to_string());
                tx_ev.send(Event::CloseConnection {stream, connection_id: key}).await?;
            }
        }
        Ok(())
    })
}

async fn broker_loop(rx_req: Sender<Request>, tx_ev: Sender<Event>, mut rx_ev: Receiver<Event>) -> Result<()> {
    let mut idx = 0;
    let mut connections: HashMap<u32, Arc<TcpStream>> = HashMap::new();
    while let Some(event) = rx_ev.next().await {
        match event {
            Event::OpenConnection {stream} => {
                debug!("Number of connected clients = {}", connections.len());
                connections.insert(idx, stream.clone());
                if let Err(err) = rx_req.send(Request{ connection_id: idx }).await {
                    panic!(err.to_string());
                }
                idx = idx.wrapping_add(1);
            },
            Event::SendingData { connection_id, payload } => {
                if let Some(stream) = connections.get_mut(&connection_id) {
                    spawn_write(write(stream.clone(), payload), tx_ev.clone(), connection_id);
                }
            },
            Event::PackageReceived {connection_id, payload} => {

            },
            Event::TimerCheckAliveConnection{} => {
                for (key, stream) in &connections {
                    spawn_read(read(stream.clone()), tx_ev.clone(), *key, stream.clone());
                }
            },
            Event::CloseConnection { stream, connection_id } => {
                match stream.peer_addr() {
                    Ok(value) => {
                        debug!("Connection {} closed by client", value);
                    }
                    Err(err) => {
                        error!("{}", err);
                    }
                }
                connections.remove(&connection_id);
            },
            Event::Disconnect {connection_id} => {
                if let Some(stream) = connections.get_mut(&connection_id) {
                    if let Ok(value) = stream.peer_addr() {
                        debug!("Closed {} connection by server", value);
                    }
                    if let Err(err) = stream.shutdown(std::net::Shutdown::Both){
                        error!("Closed connection error. {}", err.to_string());
                    }
                    connections.remove(&connection_id);
                }
            }
        }
    }
    Ok(())
}

async fn write(stream: Arc<TcpStream>, payload: Vec<u8>) -> Result<()> {
    let mut stream = &*stream;
    let header_image = format!("HTTP/1.1 200 OK\x0d\x0aConnection: close\x0d\x0aMax-Age: 0\x0d\x0aExpires: 0\x0d\x0aCache-Control: no-cache, private\x0d\x0aPragma: no-cache\x0d\x0aContent-Type: text/plan\x0d\x0aContent-Length: {}\x0d\x0a\x0d\x0a", payload.len());
    stream.write_all(header_image.as_bytes()).await?;
    stream.write_all(&payload[..]).await?;
    Ok(())
}

async fn read(stream: Arc<TcpStream>) -> Result<usize> {
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
}

async fn executor(mut rx_req: Receiver<Request>, tx_ev: Sender<Event>) -> Result<()> {
    while let Some(req) = rx_req.next().await {
        let content = String::from("Hello world!").as_bytes().iter().map(|x|*x).collect::<Vec<u8>>();
        tx_ev.send(Event::SendingData { connection_id: req.connection_id, payload: content}).await?;
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
        let mut incoming = listener.incoming().log_warnings(log_accept_error).handle_errors(Duration::from_millis(500));
        while let Some(stream) = incoming.next().await {
            if let Ok(value) = stream.peer_addr() {
                debug!("Accepted from: {}", value);
            }
            if let Err(err) = tx_ev.send(Event::OpenConnection {stream: Arc::new(stream)}).await {
                break;
            }
        }
        handle_broker.await?;
        handle_executor.await?;
        handle_timer.await?;
        Ok(())
    })
}