use std::borrow::ToOwned;
use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Formatter, Show};
use std::io::{File, IoError, IoErrorKind, IoResult};
use std::io::fs::PathExtensions;

use toml;

pub static DEFAULT_SOCKET_PATH: &'static str = "/tmp/shepherd.sock";
static CONFIG_PATHS: &'static [&'static str] = &["shepherd.toml"];

fn find_config() -> IoResult<Path> {
    for path in CONFIG_PATHS.iter() {
        let path = Path::new(path);
        
        if path.exists() {
            return Ok(path);    
        }
    }

    Err(
        IoError {
            kind: IoErrorKind::PathDoesntExist,
            desc: "Config file not found.",
            detail: Some(format!("Searched paths: {:?}", CONFIG_PATHS))
        }
    )
}

#[derive(RustcDecodable)]
struct TomlDecode {
    shepherd: Shepherd,
    servers: HashMap<String, ServerConfig>,          
}

impl TomlDecode {
    fn into_config(self) -> Config {
        let socket_path = self.shepherd.socket_path
            .unwrap_or_else(|| DEFAULT_SOCKET_PATH.to_owned());
        
        Config {
            socket_path: socket_path,
            start_servers: self.shepherd.start_servers,
            servers: self.servers,
        }    
    }    
}

#[derive(RustcDecodable)]
struct Shepherd {
    socket_path: Option<String>,
    start_servers: Vec<String>,             
}

pub struct Config {
    pub socket_path: String,
    pub start_servers: Vec<String>,
    pub servers: HashMap<String, ServerConfig>,    
}

impl Config {
    pub fn load() -> IoResult<Config> {
        let config_path = try!(find_config());
        let config_file = try!(File::open(&config_path).read_to_string());        
       
        let toml_decode = try!(opt_to_toml_res(toml::decode_str::<TomlDecode>(&*config_file)));

        Ok(toml_decode.into_config())                              
    }    
}

fn opt_to_toml_res<T>(opt: Option<T>) -> IoResult<T> {
    opt.ok_or(
        IoError {
            kind: IoErrorKind::OtherIoError,
            desc: "TOML file incorrectly formatted!",
            detail: None,
        }
    )   
}

#[derive(Clone, RustcDecodable)]
pub struct ServerConfig {
    pub dir: String,
    pub command: String,
    pub args: Vec<String>,
    pub auto_restart: Option<bool>,
    pub on_stop: Vec<String>,
    pub stop_timeout: Option<u64>,
}

impl Show for ServerConfig {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        fmt.write_fmt(format_args!("directory: {}\ncommand: {}\nargs: {:?}", self.dir, self.command, self.args))
    }
}

