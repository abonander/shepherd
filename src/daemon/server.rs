use config::ServerConfig;
use util::{FormatBytes, ignore_timeout};

use std::borrow::ToOwned;
use std::fmt::{self, Show, Formatter};
use std::io::IoResult;
use std::io::util::copy;
use std::io::process::{Command, Process};
use std::io::net::pipe::UnixStream;
use std::io::pipe::PipeStream;
use std::thread::Thread;

pub const STOP_TIMEOUT: Option<u64> = Some(10000);

pub struct Server {
   process: Process,
   config: ServerConfig,     
}

impl Server {
    pub fn spawn(config: ServerConfig) -> IoResult<Server> {
        let ref dir = Path::new(&*config.dir);
        let mut command = Command::new(&*config.command);
        let process = try!(command.cwd(dir).spawn());

        Ok(Server {
            process: process,
            config: config,
        })             
    }

    pub fn is_alive(&mut self) -> bool {
        self.process.signal(0).is_ok()    
    }

    pub fn pid(&self) -> i32 {
        self.process.id()
    }

    pub fn write_status(&mut self, w: &mut Writer) -> IoResult<()> {
        if self.is_alive() {
            writeln!(w, "Status: Running {}", ServerInfo::for_process(self.pid()))
        } else {
            w.write_line("Status: Stopped")
        }                
    }

    pub fn attach_process(&mut self, client: &mut UnixStream) {
        let mut send_server = SendServer {
            client: client.clone(),
            stdin: self.process.stdin.as_ref().unwrap().clone(),
            stdout: self.process.stdout.as_ref().unwrap().clone(),
        };

        Thread::spawn(move || {
            send_server.copy().unwrap()
        })
        .detach();     
    }

    pub fn stop(&mut self) -> IoResult<ExitStatus> {
        if !self.is_alive() {
            return Ok(ExitStatus::AlreadyStopped);    
        }
        
        self.process.set_timeout(self.config.stop_timeout.or(STOP_TIMEOUT));

        if let Some(ref on_stop) = self.config.on_stop {
            // Asking the server to stop with the given command
            try!(self.process.stdin.as_mut().unwrap().write_str(&**on_stop));
            if let Ok(_) = self.process.wait() {
                return Ok(ExitStatus::Stopped);    
            }
        }

        // Tell the server to please stop now with SIGTERM
        try!(self.process.signal_exit());
        if let Ok(_) = self.process.wait() {
            return Ok(ExitStatus::Terminated);    
        }

        // Forcibly kill the server
        try!(self.process.signal_kill());
        self.process.wait().map(|_| ExitStatus::Killed)                
    }
}

pub enum ExitStatus {
    Stopped,
    Terminated,
    Killed,
    AlreadyStopped,        
}

impl Show for ExitStatus {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        fmt.pad(match *self {
            ExitStatus::Stopped => "Stopped gently (used configured stop command)",
            ExitStatus::Terminated => "Terminated normally (no stop command or timed out)",
            ExitStatus::Killed => "Killed forcibly (server stopped responding)",
            ExitStatus::AlreadyStopped => "Server was already stopped!",
        })
    } 
}

pub struct ServerInfo {
    pub percent_cpu: f32,
    pub memory_usage: u64,
    pub uptime: String, 
}

impl ServerInfo {
    fn for_process(pid: i32) -> IoResult<ServerInfo> {
        let command = format!("ps -p {} -o %cpu,rss,etime --no-headers", pid);

        let output = try!(Command::new(command).output());
        let output = String::from_utf8_lossy(&*output.output);
       
        let columns: Vec<String> = output.words().map(ToOwned::to_owned).collect();
       
        Ok(ServerInfo {
           percent_cpu: columns[0].parse().unwrap(),
           memory_usage: columns[1].parse().unwrap(),
           uptime: columns[2].clone(),
        })             
    }
}

impl Show for ServerInfo {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        fmt.write_fmt(
            format_args!(
                "CPU: {}% Memory: {} Uptime: {}", 
                self.percent_cpu,
                FormatBytes(self.memory_usage),
                self.uptime
            )
        )
    } 
}

struct SendServer {
    client: UnixStream,
    stdin: PipeStream,
    stdout: PipeStream,
}

impl SendServer {
    fn copy(&mut self) -> IoResult<()> {
        loop {
            try!(ignore_timeout(copy(&mut self.client, &mut self.stdin)));
            try!(ignore_timeout(copy(&mut self.stdout, &mut self.client)));
        }    
    }  
}

unsafe impl Send for SendServer {}

