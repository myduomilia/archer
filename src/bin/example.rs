extern crate vms;
use env_logger::Env;

fn main() {
    env_logger::init_from_env(Env::default().default_filter_or("debug"));
    vms::run("127.0.0.1:8080").unwrap();
}