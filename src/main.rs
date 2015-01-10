#![allow(unstable)]

extern crate libc;
extern crate time;
extern crate toml;
extern crate "rustc-serialize" as rustc_serialize;

use daemon::RemoteDaemon;

use std::io::{IoResult, stdio};
use std::os;

mod config;
mod daemon;
mod util;

const MAX_TIMEOUTS: u32 = 200;

fn main() {
    let mut args = os::args();

    let command = args.remove(0);

    let config = config::Config::load().unwrap();
 
    if let Some(op) = args.get(0) {
        if &**op == "start-daemon" {
            daemon::start(config);
            return;
        }
    } 
    
    let mut daemon = RemoteDaemon::connect_or_spawn(&*command, &*config.socket_path).unwrap();

    stdio::println("Connected to daemon.");

    if args.is_empty() {    
        if command_loop(daemon).is_err() {
            stdio::println("\nServer closed connection.");
        } else {
            stdio::println("\nExiting...");
        }
    } else {
        let ref mut stdout = stdio::stdout();
        daemon.send_command(&*args.connect(" ")).unwrap();
        daemon.write_response(stdout, MAX_TIMEOUTS).unwrap();
    }
}

fn command_loop(mut daemon: RemoteDaemon) -> IoResult<()> {
    let mut stdout = stdio::stdout();

    try!(stdout.write_str("> ").and_then(|_| stdout.flush()));
    for line in stdio::stdin().lock().lines().filter_map(|line| line.ok()) {
        try!(daemon.send_command(&*line));
        try!(daemon.write_response(&mut stdout, MAX_TIMEOUTS));
        try!(stdout.write_str("> ").and_then(|_| stdout.flush()));        
    }
    
    Ok(()) 
}

