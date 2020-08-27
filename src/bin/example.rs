extern crate mjpeg_server;
#[macro_use]
extern crate log;

use env_logger::Env;
use mjpeg_server::MjpegServer;

fn main() {
    env_logger::init_from_env(Env::default().default_filter_or("debug"));

    match MjpegServer::new() {
        Ok(mut server) => {
            loop {
                if let Err(err) = server.run("127.0.0.1:8080") {
                    error!("{:?}", err);
                }
            }
        }
        Err(err) => {
            error!("{:?}", err);
        }
    }
}