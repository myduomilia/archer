use async_std::net::{TcpListener, TcpStream};
use async_std::io::BufReader;
use async_std::prelude::*;
use async_std::task;
use log::{debug, error, info};
use std::time::Duration;
use std::{thread, fs};
use futures::channel::mpsc;
use futures::sink::SinkExt;
use futures::{select, FutureExt, StreamExt};
use std::thread::sleep;
use std::sync::Arc;
use std::option::Option::Some;
use std::collections::{BTreeSet, BTreeMap, HashMap};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::UnboundedSender<T>;
type Receiver<T> = mpsc::UnboundedReceiver<T>;

enum Event {
    NewPeer {
        stream: Arc<TcpStream>,
    },
    Message {
        msg: Arc<Vec<u8>>,
    },
}

fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
    where
        F: Future<Output = Result<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            error!("{:?}", e)
        }
    })
}

async fn broker_loop(mut rx: Receiver<Event>) -> Result<()> {
    let mut idx = 0;
    let mut writers = Vec::new();
    let mut senders: BTreeMap<i64, Sender<Arc<Vec<u8>>>> = BTreeMap::new();
    while let Some(event) = rx.next().await {
        match event {
            Event::NewPeer{stream} => {
                let (tx, rx): (Sender<Arc<Vec<u8>>>, Receiver<Arc<Vec<u8>>>) =  mpsc::unbounded();
                senders.insert(idx, tx);
                idx = idx.wrapping_add(1);
                let handle = spawn_and_log_error(send_loop(rx, stream));
                writers.push(handle);
                debug!("Control event received");
            },
            Event::Message{msg} => {
                let mut remove_keys = vec![];
                for (idx, client) in &mut senders {
                    if let Err(err) = client.send(msg.clone()).await {
                        error!("{:?}", err);
                        remove_keys.push(*idx);
                    }
                }
                for id in remove_keys {
                    senders.remove(&id);
                }
                debug!("Data event received");
            }
        }
        debug!("{}", senders.len());
    }
    for writer in writers {
        writer.await;
    }
    Ok(())
}

async fn send_loop(mut rx: Receiver<Arc<Vec<u8>>>, stream: Arc<TcpStream>) -> Result<()> {
    let mut stream = &*stream;
    while let Some(msg) = rx.next().await {
        let header_image = format!("HTTP/1.0 200 OK\x0d\x0aConnection: close\x0d\x0aMax-Age: 0\x0d\x0aExpires: 0\x0d\x0aCache-Control: no-cache, private\x0d\x0aPragma: no-cache\x0d\x0aContent-Type: image/jpeg\x0d\x0aContent-Length: {}\x0d\x0a\x0d\x0a", msg.len());
        stream.write_all(header_image.as_bytes()).await?;
        stream.write_all(&msg[..]).await?;
    }
    Ok(())
}

async fn capture(mut tx: Sender<Event>) -> Result<()> {
    loop {
        let contents = fs::read("./image.jpg")?;
        if let Err(err) = tx.send(Event::Message {msg: Arc::new(contents)}).await {
            panic!("{:?}", err);
        }
        sleep(Duration::from_secs(1));
    }
}

pub fn run(address: &str) -> Result<()> {
    let (mut tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::unbounded();
    task::block_on(async move {
        let handle = task::spawn(capture(tx.clone()));
        let broker = task::spawn(broker_loop(rx));
        let listener = TcpListener::bind(address).await?;
        info!("Listening on {}", address);
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            debug!("Accepted from: {}", stream.peer_addr()?);
            tx.send(Event::NewPeer {stream: Arc::new(stream)}).await.unwrap();
        }
        broker.await?;
        handle.await?;
        Ok(())
    })
}