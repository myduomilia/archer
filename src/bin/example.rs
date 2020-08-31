extern crate archers;
use env_logger::Env;
use log::{error};

fn main() {
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    if let Err(err) = archers::run("0.0.0.0:8080") {
        error!("{}", err.to_string());
    }
}