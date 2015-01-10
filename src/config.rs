use std::borrow::ToOwned;
use std::collections::HashMap;
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


pub struct Config {
    pub socket_path: String,
    pub servers: HashMap<String, ServerConfig>,    
}

impl Config {
    pub fn load() -> IoResult<Config> {
        let config_path = try!(find_config());
        let config_file = try!(
            File::open(&config_path).and_then(|mut file| file.read_to_string())
        );        
        
        let config_toml = try!(opt_to_toml_res(config_file.parse::<toml::Value>()));

        let socket_path = config_toml.lookup("shepherd.socket_path")
            .and_then(toml::Value::as_str)
            .unwrap_or(DEFAULT_SOCKET_PATH)
            .to_owned();

        let mut servers = HashMap::new();

        for (key, value) in try!(
            opt_to_toml_res(config_toml.lookup("servers")
                .and_then(toml::Value::as_table)
            )).to_owned().into_iter() 
        {
            let value = try!(opt_to_toml_res(toml::decode::<ServerConfig>(value)));

            servers.insert(key, value);              
        }

        Ok(Config {
            socket_path: socket_path,
            servers: servers,
        })                              
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
    pub auto_restart: Option<bool>,
    pub on_stop: Option<String>,
    pub stop_timeout: Option<u64>,
}

impl Show for ServerConfig {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        fmt.write_fmt(format_args!("directory: {}\ncommand: {}", self.dir, self.command))
    }
}

