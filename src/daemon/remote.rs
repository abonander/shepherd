use super::ClientStream;
use util::ignore_timeout;

use std::io::{self, IoResult};
use std::io::process::{Command, StdioContainer};
use std::io::net::pipe::UnixStream;
use std::io::timer::sleep;
use std::time::Duration;
use std::io::util::copy;

pub struct RemoteDaemon {
    stream: ClientStream,        
}

impl RemoteDaemon {
    fn connect(socket: &str) -> IoResult<RemoteDaemon> {
        let mut client_stream = try!(UnixStream::connect(socket));
        client_stream.set_timeout(Some(5000));

        Ok(RemoteDaemon {
            stream: super::buffered(client_stream)
        })            
    }
        
    pub fn connect_or_spawn(command: &str, socket: &str) -> IoResult<RemoteDaemon> {
        if let Ok(daemon) =  RemoteDaemon::connect(socket) {
            return Ok(daemon);    
        }
       
        io::stdio::println("Daemon not running! Starting...");         
        try!(spawn_daemon(command));
        sleep(Duration::seconds(1));
        io::stdio::println("Daemon started.");
        
        RemoteDaemon::connect(socket)         
    }
    
    pub fn send_command(&mut self, command: &str) -> IoResult<()> {
        try!(self.stream.write_line(command));
        self.stream.flush()
    }

    pub fn write_response<W: Writer>(&mut self, w: &mut W) -> IoResult<()> {
        ignore_timeout(copy(&mut self.stream, w)).map(|_| ())
    }
}


fn spawn_daemon(command: &str) -> IoResult<()> {
    let mut command = Command::new(command);
    command.arg("start-daemon")
        .stdin(StdioContainer::Ignored)
        .stdout(StdioContainer::Ignored)
        .stderr(StdioContainer::Ignored)
        .detached();

    Ok(try!(command.spawn()).forget()) 
}

