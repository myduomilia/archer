use std::error::Error;
use std::fmt;

use async_std::io;
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::task;


use log::{debug, info, warn};
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

pub struct MjpegServerError<'a>{
    description: &'a str,
}

impl fmt::Display for MjpegServerError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description)
    }
}

impl fmt::Debug for MjpegServerError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{ file: {}, line: {}, message: {} }}", file!(), line!(), self.description)
    }
}

pub struct MjpegServer {

}

impl MjpegServer {

    pub fn new<'a>() -> Result<Self, MjpegServerError<'a>> {
        Ok(MjpegServer{})
        // Err(MjpegServerError{description: "Can't create a server"})
    }

    pub fn run(&mut self, address: &str) -> Result<(), Box<dyn Error>> {
        task::block_on(async {
            let listener = TcpListener::bind(address).await?;
            let broker = task::spawn(async {
                loop {
                    sleep(Duration::from_secs(5));
                    debug!("ups");
                }
            });
            info!("Listening on {}", address);
            let mut incoming = listener.incoming();
            while let Some(stream) = incoming.next().await {
                let stream = stream?;
                debug!("Accepted from: {}", stream.peer_addr()?);
            }
            broker.await;
            Ok(())
        })
    }
}