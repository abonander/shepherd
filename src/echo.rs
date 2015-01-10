use std::io::stdio::{stdin, stdout};

fn main() {
    let mut stdout = stdout();
    for line in stdin().lock().lines().filter_map(|line| line.ok()) {
        stdout.write_line(&*line).unwrap();    
    }    
}
