use super::ClientStream;
use util::ignore_timeout;

use std::io::{self, IoResult};
use std::io::process::{Command, StdioContainer};
use std::io::net::pipe::UnixStream;
use std::io::timer::sleep;
use std::io::util::copy;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::thread::Thread;
use std::time::Duration;

pub struct RemoteDaemon {
    stream: ClientStream,
    timeout_thread: TimeoutThread,  
}

impl RemoteDaemon {
    fn connect(socket: &str) -> IoResult<RemoteDaemon> {
        let client_stream = try!(UnixStream::connect_timeout(socket, Duration::milliseconds(2000)));

        let timeout_thread = TimeoutThread::new(Duration::seconds(5));

        Ok(RemoteDaemon {
            stream: super::buffered(client_stream),
            timeout_thread: timeout_thread,
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
        self.timeout_thread.start();
        try!(self.stream.write_line(command));
        let ret = self.stream.flush();
        self.timeout_thread.stop();
        ret
    }

    pub fn write_response<W: Writer>(&mut self, w: &mut W, max_timeouts: u32) -> IoResult<()> {
        let mut timeouts = 0u32;
        self.stream.get_mut().set_timeout(Some(200));

        loop {
            self.timeout_thread.start();
            if try!(ignore_timeout(copy(&mut self.stream, w))).is_none() {
                timeouts += 1;

                if timeouts > max_timeouts {
                    break;    
                }
            }

            self.timeout_thread.stop();
        }
        self.timeout_thread.stop();

        w.flush()
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

struct TimeoutThread {
    thread: Thread,
    timeout_bool: Arc<AtomicBool>,
}

impl TimeoutThread {

    fn new(timeout: Duration) -> TimeoutThread {
        let timeout_bool = Arc::new(AtomicBool::new(false));
        let ret_timeout_bool = timeout_bool.clone();

        let thread = Thread::spawn(move || {
            loop {
                Thread::park();
                sleep(timeout);

                if timeout_bool.load(Relaxed) {
                    use std::intrinsics;

                    println!("Connection timed out!");
                    unsafe { intrinsics::abort(); }                      
                }

            }
        });

        TimeoutThread {
            thread: thread,
            timeout_bool: ret_timeout_bool
        }  
    }

    fn start(&self) {
        self.timeout_bool.store(true, Relaxed);
        self.thread.unpark();   
    }

    fn stop(&self) {
        self.timeout_bool.store(false, Relaxed);    
    }
}
