use config::Config;
use util::ignore_timeout;

pub use self::remote::RemoteDaemon;

use self::server::Server;

use std::borrow::ToOwned;
use std::collections::hash_map::{Entry, HashMap};
use std::error::FromError;
use std::io::{Acceptor, BufferedStream, IoError, Listener};
use std::io::fs::{self, PathExtensions};
use std::io::net::pipe::{UnixAcceptor, UnixListener, UnixStream};

mod remote;
mod server;

pub const TIMEOUT: Option<u64> = Some(100);

pub fn start(config: Config) {
    println!("Starting daemon... Type Ctrl-z to stop and `bg` to run in backround");

    let ref socket_path = Path::new(&*config.socket_path);
    
    if socket_path.exists() {
        fs::unlink(socket_path).ok()
            .expect("Socket already exists and cannot be deleted! Is a daemon already running?");   
    } 

    let listener = UnixListener::bind(socket_path).unwrap();

    let mut acceptor = listener.listen().unwrap();
    // 100ms timeout
    acceptor.set_timeout(TIMEOUT);

    let mut daemon = Daemon::new(config, acceptor);
    let mut clients: Vec<ClientStream> = Vec::new();

    daemon.start_servers();

    while daemon.manage_clients(&mut clients) {
        daemon.check_instances();
    }

    for mut client in clients.into_iter() {
        let client = client.get_mut();
        if let Err(err) = client.close_write().and_then(|_| client.close_read()) {
            println!("Error closing client: {}", err);    
        }
    }

    daemon.acceptor.close_accept().unwrap();
}

pub type ClientStream = BufferedStream<UnixStream>;

pub fn buffered(stream: UnixStream) -> ClientStream {
    BufferedStream::new(stream)    
}

struct Daemon {
    config: Config,
    acceptor: UnixAcceptor,
    servers: HashMap<String, Server>,
}

impl Daemon {
    fn new(config: Config, acceptor: UnixAcceptor) -> Daemon {
        Daemon {
            config: config,
            acceptor: acceptor,
            servers: HashMap::new(),
        }    
    }

    fn start_servers(&mut self) {
        for server in self.config.start_servers.iter().cloned() {
            if let Some(config) = self.config.servers.get(&*server).cloned() {
                println!("Auto-starting \"{}\"...", server);
                match Server::spawn(config) {
                    Ok(instance) => {
                        println!("\"{}\" started!", server);
                        self.servers.insert(server, instance);
                    },
                    Err(err) => println!("Error auto-starting \"{}\": {}", server, err),                       
                }
            } else {
                println!("No config found for \"{}\"", server);    
            }
        }            
    }

    fn manage_clients(&mut self, clients: &mut Vec<ClientStream>) -> bool {
        while let Ok(Some(client)) = ignore_timeout(self.acceptor.accept()) {
            println!("Client connected!");
            clients.push(buffered(client));
        }

        let mut closed = Vec::new();

        for (idx, client) in clients.iter_mut().enumerate() {
            if let Ok(Some(command)) = ignore_timeout(client.read_line()) {
                if command.trim().is_empty() { continue; }

                let args: Vec<_> = command.words().map(ToOwned::to_owned).collect();
                println!("Received command: {:?}", args);

                match self.match_op(client, args) {
                    Err(ClientError::Io(err)) => println!("Client IO Error: {}", err),
                    Err(ClientError::Killed) => return false,
                    Ok(_) => (), 
                }
            } else {
                println!("Client closed connection!");
                closed.push(idx);
            }                 
        }

        for idx in closed.into_iter() {
            clients.remove(idx);    
        }

        true
    }

    fn check_instances(&mut self) {
        let mut to_remove = Vec::new();

        for (server, instance) in self.servers.iter_mut() {
            if !instance.is_alive() && instance.auto_restart() {
                println!("\"{}\" has died!\nLast five lines of log:", server);
                for line in instance.tail(5).iter() {
                    println!("{}", line);    
                }

                if let Some(config) = self.config.servers.get(server).cloned() {
                    println!("Restarting \"{}\"...", server);
                    *instance = match Server::spawn(config) {
                        Ok(new_instance) => new_instance,
                        Err(err) => {
                            println!("Error restarting \"{}\": {}", server, err);
                            continue;
                        }
                    }
                } else {
                    println!("Lost config for \"{}\"!", server);
                    to_remove.push(server.clone());    
                }                 
            }
        }

        for server in to_remove.iter() {
            self.servers.remove(server);    
        }
    }

    fn match_op(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        if !args.is_empty() {
            let op = args.remove(0);

            match &*op {
                "start" => self.start_server(client, args),
                "stop" => self.stop_server(client, args),
                "restart" => self.restart_server(client, args),
                "status" => self.server_status(client, args),
                "tail" => self.server_tail(client, args),
                "send" => self.server_send(client, args),
                "servers" => self.list_servers(client),
                "instances"=> self.list_instances(client),
                "reload-config" => self.reload_config(client),
                "kill-daemon" => self.kill_daemon(client),
                "ops" => list_ops(client),
                _ => {
                    ce(writeln!(client, "Unrecognized command: {}", op))
                        .and_then(|_| list_ops(client))
                },
            }
        } else {
            list_ops(client)    
        }.and_then(|&mut: _| ce(client.flush()))
    }

    fn start_server(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        if args.is_empty() {
            return ce(client.write_line("Usage: start <server>"));        }

        let server = args.remove(0);
                
        ce(match self.servers.entry(server.clone()) {
            Entry::Occupied(_) => writeln!(client, "Server \"{}\" already running!", server),
            Entry::Vacant(vacant) => {
                if let Some(config) = self.config.servers.get(&*server) {
                    try!(writeln!(client, "Starting \"{}\"...", server).and_then(|_| client.flush()));
                    match Server::spawn(config.clone()) {
                        Ok(instance) => {
                            vacant.insert(instance);
                            writeln!(client, "Server \"{}\" started!", server)
                        },
                        Err(err) => writeln!(client, "Error starting \"{}\": {:?}", server, err),
                    }
                } else {
                    writeln!(client, "No configuration for \"{}\"", server)
                }
            }
        })
    }

    fn stop_server(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        if args.is_empty() {
            return ce(client.write_line("Usage: stop <server>"));
        }

        let server = args.remove(0);

        if let Some(mut instance) = self.servers.remove(&*server) {
            stop_server(&*server, &mut instance, client).map(|_| ())            
        } else {
            ce(writeln!(client, "No running instance of \"{}\"", server))
        }
    }

    fn restart_server(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        if args.is_empty() {
            return ce(client.write_line("Usage: restart <server>"));
        }

        let server = args.remove(0);

        if let Some(mut instance) = self.servers.remove(&*server) {
            if !try!(stop_server(&*server, &mut instance, client)) {
                return Ok(());  
            }
        } else {
            try!(writeln!(client, "\"{}\" was not running! Starting anyways...", server));
        }
         
        // Start server
        ce(if let Some(config) = self.config.servers.get(&*server) {
            let instance = try!(Server::spawn(config.clone()));
            let res = writeln!(client, "Server \"{}\" started!", server);
            self.servers.insert(server, instance);
            res
        } else {
            writeln!(client, "No configuration for \"{}\"! Did the configuration change?", server)
        })
    }

    fn server_status(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        if args.is_empty() {
            return ce(client.write_line("Usage: status <server>")); 
        }

        let server = args.remove(0);

        ce(if let Some(instance) = self.servers.get_mut(&*server) {
            try!(writeln!(client, "Server instance \"{}\" ", server));
            instance.write_status(client)
        } else {
            writeln!(client, "No running instance of \"{}\"", server)
        })
    }

    fn server_tail(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        const DEFAULT_LINE_COUNT: usize = 20;
        
        if args.is_empty() {
            return ce(client.write_line("Usage: tail <server> [lines]"));
        }

        let server = args.remove(0);
        let lines = args.get(0).and_then(|s| s.parse()).unwrap_or(DEFAULT_LINE_COUNT);

        if let Some(instance) = self.servers.get_mut(&*server) {
            try!(writeln!(client, "Last {} lines from \"{}\":", lines, server).and_then(|_| client.flush()));
            for line in instance.tail(lines).iter() {
                try!(client.write_str(&**line));    
            }

            Ok(())
        } else {
            ce(writeln!(client, "No running instance of \"{}\"", server))
        }
    }

    fn server_send(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> ClientResult<()> {
        if args.is_empty() {
            return ce(client.write_line("Usage: send <server> <command>"));
        }

        let server = args.remove(0);
        let command = args.connect(" ");

        ce(if let Some(instance) = self.servers.get_mut(&*server) {
            try!(writeln!(client, "Sending command to \"{}\": {}", server, command).and_then(|_| client.flush()));
            try!(instance.send_command(&*command));
            if let Some(line) = instance.tail(1).get(0) {
                writeln!(client, "\"{}\": {}", server, line)
            } else {
                Ok(())
            }
        } else {
            writeln!(client, "No running instance of \"{}\"", server)
        })
    }

    fn list_servers(&mut self, client: &mut ClientStream) -> ClientResult<()> {
        try!(client.write_line("Servers:"));
        for (server, config) in self.config.servers.iter() {
            try!(writeln!(client, "\"{}\":\n{:?}", server, config));    
        }
        Ok(())
    }

    fn list_instances(&mut self, client: &mut ClientStream) -> ClientResult<()> {
        try!(client.write_line("Running instances:"));
        for (server, instance) in self.servers.iter_mut() {
            try!(write!(client, "\"{}\" ", server));
            try!(instance.write_status(client));    
        }
        Ok(())        
    }

    fn reload_config(&mut self, client: &mut ClientStream) -> ClientResult<()> {
        try!(
            client.write_line("Reloading config. Will not affect existing server instances.")
                .and_then(|_| client.flush())
        );

        self.config = match Config::load() {
            Ok(config) => config,
            Err(err) => return ce(writeln!(client, "Failed to load config: {:?}", err)),
        };

        ce(client.write_line("Config reloaded."))
    }

    fn kill_daemon(&mut self, client: &mut ClientStream) -> ClientResult<()> { 
        try!(client.write_line("Killing servers..."));
        for (server, mut instance) in self.servers.drain() {
            let _  = stop_server(&*server, &mut instance, client);
            try!(client.flush())
        }

        try!(
            client.write_line("Daemon exiting. Any servers that failed to stop will die now.")
                .and_then(|_| client.flush())
                .and_then(|_| client.write(&[0; 64]))
        );

        Err(ClientError::Killed)
    }
}

fn stop_server(server: &str, instance: &mut Server, client: &mut ClientStream) -> ClientResult<bool> {
    try!(writeln!(client, "Sending stop command to \"{}\"...", server).and_then(|_| client.flush()));
    ce(match instance.stop() {
        Ok(exit_status) => writeln!(client, "\"{}\" stopped. Status: {}", server, exit_status)
            .map(|_| true),
        Err(err) => writeln!(client, "Failed to stop \"{}\"! Message: {}", server, err)
            .map(|_| false),
    })
}

fn list_ops(writer: &mut Writer) -> ClientResult<()> {
    ce(writer.write_line(r#"
shepherd ops:
    start <server>
    stop <server> 
    restart <server>
    tail <server> [lines]
    send <server> <commandt
    status <server>
    servers
    instances
    ops
    kill-daemon
"#))  
}

type ClientResult<T> = Result<T, ClientError>;

enum ClientError {
    Io(IoError),
    Killed,    
}

impl FromError<IoError> for ClientError {
    #[inline]
    fn from_error(e: IoError) -> ClientError {
        ClientError::Io(e)
    }
}

#[inline]
fn ce<T>(res: Result<T, IoError>) -> ClientResult<T> {
    res.map_err(FromError::from_error)    
}


