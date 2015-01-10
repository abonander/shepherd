use config::ServerConfig;
use util::{FormatBytes, FormatTime, precise_time_ms};

use std::borrow::ToOwned;
use std::fmt;
use std::io::{BufferedReader, File, IoResult};
use std::io::process::{Command, Process};
use std::io::pipe::PipeStream;
use std::sync::mpsc::{sync_channel, Receiver};
use std::thread::Thread;

pub const STOP_TIMEOUT: Option<u64> = Some(10000);

pub struct Server {
   process: Process,
   config: ServerConfig,
   lines: Receiver<String>,
   log: Vec<String>, 
}

impl Server {
    pub fn spawn(config: ServerConfig) -> IoResult<Server> {
        let ref dir = Path::new(&*config.dir);
        let mut command = Command::new(&*config.command);
        
        for arg in config.args.iter() {
            command.arg(arg);    
        }

        command.cwd(dir);
        
        println!("Starting process: {}", command);

        let process = try!(command.spawn());

        let lines = read_lines_threaded(process.stdout.clone().unwrap());

        Ok(Server {
            process: process,
            config: config,
            lines: lines,
            log: Vec::new(),
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
            writeln!(w, "Status: Running [{}]", try!(ServerInfo::for_process(self.pid())))
        } else {
            w.write_line("Status: Stopped")
        }                
    }
 
    pub fn stop(&mut self) -> IoResult<ExitStatus> {
        if !self.is_alive() {
            return Ok(ExitStatus::AlreadyStopped);    
        }
        
        self.process.set_timeout(self.config.stop_timeout.or(STOP_TIMEOUT));

        if !self.config.on_stop.is_empty() {
            // Asking the server to stop with the given command

            for command in self.config.on_stop.clone().into_iter() {
                try!(self.send_command(&*command));
            }

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

    pub fn send_command(&mut self, command: &str) -> IoResult<()> {
        let stdin = self.process.stdin.as_mut().unwrap();
        stdin.write_line(command).and_then(|_| stdin.flush())
    }

    pub fn read_line(&mut self, timeout_ms: u64) -> Option<String> {
        let start = precise_time_ms();

        while (precise_time_ms() - start) < timeout_ms {
            if let Ok(line) = self.lines.try_recv() {
                return Some(line);
            }          
        }

        None
    }

    pub fn tail(&mut self, lines: usize) -> &[String] {
        while let Some(line) = self.read_line(100) {
            self.log.push(line);     
        }
        
        truncate_back(&mut self.log, 80);
        
        let offset = if lines > self.log.len() {
            0    
        } else {
            self.log.len() - lines    
        };

        &self.log[offset..]
    }
    
    pub fn auto_restart(&self) -> bool {
        self.config.auto_restart.unwrap_or(false) 
    }
}

pub enum ExitStatus {
    Stopped,
    Terminated,
    Killed,
    AlreadyStopped,        
}

impl fmt::String for ExitStatus {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt.pad(match *self {
            ExitStatus::Stopped => "Stopped gently (used configured stop command)",
            ExitStatus::Terminated => "Terminated normally (no stop command or timed out)",
            ExitStatus::Killed => "Killed forcibly (server stopped responding)",
            ExitStatus::AlreadyStopped => "Server was already stopped!",
        })
    } 
}

pub struct ServerInfo {
    pub pid: i32,
    pub percent_cpu: f32,
    pub memory_usage: u64,
    pub uptime: u64, 
}

impl ServerInfo {
    fn for_process(pid: i32) -> IoResult<ServerInfo> {
        fn ticks_per_second() -> u64 {
            use libc::sysconf;
            use libc::consts::os::sysconf::_SC_CLK_TCK;

            unsafe { sysconf(_SC_CLK_TCK) as u64 }
        }

        fn ticks_to_s(ticks: u64) -> u64 {
            ticks / ticks_per_second()    
        }

        let ref proc_uptime_path = Path::new("/proc/uptime");
        let proc_uptime = try!(File::open(proc_uptime_path).read_to_string());
        let uptime = proc_uptime.words().next().and_then(|s| s.parse::<f64>()).unwrap() as u64;

        let ref proc_stat_path = Path::new(format!("/proc/{}/stat", pid));
        let proc_stat = try!(File::open(proc_stat_path).read_to_string());

        // Indexes into `columns` taken from the man page for /proc/[pid]/stat
        // Mind that the man page starts the indexes from 1 but these start from 0
        let columns: Vec<_> = proc_stat.words().map(ToOwned::to_owned).collect();

        // Get kernel time and user time
        let cputime_ticks: u64 = columns[13].parse::<u64>().unwrap() + columns[14].parse().unwrap();
        let cputime = ticks_to_s(cputime_ticks);
 
        let start_time = ticks_to_s(columns[21].parse().unwrap());
        let runtime = uptime - start_time;

        let percent_cpu = (cputime as f32) / (runtime as f32);
       
        Ok(ServerInfo {
            pid: pid,
            percent_cpu: percent_cpu,
            memory_usage: columns[23].parse().unwrap(),
            uptime: runtime,
        })             
    }
}

impl fmt::String for ServerInfo {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt.write_fmt(
            format_args!(
                "PID: {} Avg CPU: {:.02}% Memory: {} Uptime: {}",
                self.pid,
                self.percent_cpu,
                FormatBytes(self.memory_usage),
                FormatTime::from_s(self.uptime)
            )
        )
    } 
}

const MAX_LINES: usize = 80;

fn read_lines_threaded(stream: PipeStream) -> Receiver<String> {
    let (tx, rx) = sync_channel(MAX_LINES);

    Thread::spawn(move || {
        let mut reader = BufferedReader::new(stream);
        for line in reader.lines() {
            let line = line.unwrap();

            if !line.trim().is_empty() {
                tx.send(line).unwrap();
            }           
        }
    });

    rx
}

fn truncate_back<T>(vec: &mut Vec<T>, len: usize) {
    let offset = if vec.len() > len {
        vec.len() - len    
    } else {
        return;    
    };

    let ptr = vec[offset..].as_ptr();
    unsafe {
        vec.set_len(offset);
        *vec = Vec::from_raw_buf(ptr, len);
    }
}
