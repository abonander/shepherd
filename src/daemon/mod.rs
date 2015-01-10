use config::Config;
use util::ignore_timeout;

pub use self::remote::RemoteDaemon;

use self::server::Server;

use std::borrow::ToOwned;
use std::collections::hash_map::{Entry, HashMap};
use std::io::{Acceptor, BufferedStream, IoResult, Listener};
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
    let ref mut clients: Vec<ClientStream> = Vec::new();

    loop {
        daemon.manage_clients(clients);
    }  
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

    fn manage_clients(&mut self, clients: &mut Vec<ClientStream>) {
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

                if let Err(err) = self.match_op(client, args) {
                    println!("{}", err);    
                }
            } else {
                println!("Client closed connection!");
                closed.push(idx);
            }                 
        }

        for idx in closed.into_iter() {
            clients.remove(idx);    
        }
    }

    fn match_op(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
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
                    writeln!(client, "Unrecognized command: {}", op)
                        .and_then(|_| list_ops(client))
                },
            }
        } else {
            list_ops(client)    
        }.and_then(|&mut: _| client.flush())
    }

    fn start_server(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
        if args.is_empty() {
            return client.write_line("Usage: start <server>");
        }

        let server = args.remove(0);
                
        match self.servers.entry(server.clone()) {
            Entry::Occupied(_) => writeln!(client, "Server \"{}\" already running!", server),
            Entry::Vacant(vacant) => {
                if let Some(config) = self.config.servers.get(&*server) {
                    try!(writeln!(client, "Starting \"{:?}\"...", server).and_then(|_| client.flush()));
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
        }
    }

    fn stop_server(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
        if args.is_empty() {
            return client.write_line("Usage: stop <server>");
        }

        let server = args.remove(0);

        if let Some(mut instance) = self.servers.remove(&*server) {
            stop_server(&*server, &mut instance, client).map(|_| ())            
        } else {
            writeln!(client, "No running instance of \"{}\"", server)    
        }
    }

    fn restart_server(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
        if args.is_empty() {
            return client.write_line("Usage: restart <server>");
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
        if let Some(config) = self.config.servers.get(&*server) {
            let instance = try!(Server::spawn(config.clone()));
            let res = writeln!(client, "Server \"{}\" started!", server);
            self.servers.insert(server, instance);
            res
        } else {
            writeln!(client, "No configuration for \"{}\"! Did the configuration change?", server)
        }
    }

    fn server_status(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
        if args.is_empty() {
            return client.write_line("Usage: status <server>"); 
        }

        let server = args.remove(0);

        if let Some(instance) = self.servers.get_mut(&*server) {
            instance.write_status(client)  
        } else {
            writeln!(client, "No running instance of \"{}\"", server)  
        }
    }

    fn server_tail(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
        const DEFAULT_LINE_COUNT: usize = 20;
        
        if args.is_empty() {
            return client.write_line("Usage: tail <server> [lines]");    
        }

        let server = args.remove(0);
        let lines = args.get(0).and_then(|s| s.parse()).unwrap_or(DEFAULT_LINE_COUNT);

        if let Some(instance) = self.servers.get_mut(&*server) {
            try!(writeln!(client, "Last {} lines from \"{}\":", lines, server).and_then(|_| client.flush()));
            for line in instance.tail(lines).into_iter() {
                try!(client.write_str(&*line));    
            }

            Ok(())
        } else {
            writeln!(client, "No running instance of \"{}\"", server)    
        }
    }

    fn server_send(&mut self, client: &mut ClientStream, mut args: Vec<String>) -> IoResult<()> {
        if args.is_empty() {
            return client.write_line("Usage: send <server> <command>");    
        }

        let server = args.remove(0);
        let command = args.connect(" ");

        if let Some(instance) = self.servers.get_mut(&*server) {
            try!(writeln!(client, "Sending command to \"{}\": {}", server, command).and_then(|_| client.flush()));
            try!(instance.send_command(&*command));
            if let Some(line) = instance.tail(1).get(0) {
                writeln!(client, "\"{}\": {}", server, line)
            } else {
                Ok(())
            }
        } else {
            writeln!(client, "No running instance of \"{}\"", server)    
        }
    }

    fn list_servers(&mut self, client: &mut ClientStream) -> IoResult<()> {
        try!(client.write_line("Servers:"));
        for (server, config) in self.config.servers.iter() {
            try!(writeln!(client, "\"{}\":\n{:?}", server, config));    
        }
        Ok(())
    }

    fn list_instances(&mut self, client: &mut ClientStream) -> IoResult<()> {
        try!(client.write_line("Running instances:"));
        for (server, instance) in self.servers.iter_mut() {
            try!(write!(client, "\"{}\" ", server));
            try!(instance.write_status(client));    
        }
        Ok(())        
    }

    fn reload_config(&mut self, client: &mut ClientStream) -> IoResult<()> {
        try!(
            client.write_line("Reloading config. Will not affect existing server instances.")
                .and_then(|_| client.flush())
        );

        self.config = match Config::load() {
            Ok(config) => config,
            Err(err) => return writeln!(client, "Failed to load config: {:?}", err),
        };

        client.write_line("Config reloaded.")
    }

    fn kill_daemon(&mut self, client: &mut ClientStream) -> IoResult<()> { 
        try!(client.write_line("Killing servers..."));
        for (server, mut instance) in self.servers.drain() {
            let _  = stop_server(&*server, &mut instance, client);
            try!(client.flush())
        }

        try!(
            client.write_line("Daemon exiting. Any servers that failed to stop will die now.")
                .and_then(|&mut: _| client.flush())
        );

        
        panic!("Daemon exiting!");
    }
}

fn stop_server(server: &str, instance: &mut Server, client: &mut ClientStream) -> IoResult<bool> {
    try!(writeln!(client, "Sending stop command to \"{}\"...", server).and_then(|_| client.flush()));
    match instance.stop() {
        Ok(exit_status) => writeln!(client, "\"{}\" stopped. Status: {}", server, exit_status)
            .map(|_| true),
        Err(err) => writeln!(client, "Failed to stop \"{}\"! Message: {}", server, err)
            .map(|_| false),
    } 
}

fn list_ops(writer: &mut Writer) -> IoResult<()> {
    writer.write_line(r#"
shepherd ops:
    start <server>
    stop <server> 
    restart <server>
    trim <server>
    attach <server>
    status <server>
    servers
    instances
    ops
    kill-daemon
"#)   
}

