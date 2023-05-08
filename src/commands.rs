use defmt::*;
use prse::Parse;

#[derive(Parse, Debug, PartialEq)]
pub enum Command {
    #[prse = "wait {}"]
    Wait(u32),
    #[prse = "move {x} {y}"]
    MoveMouse { x: i32, y: i32 },
    #[prse = "click"]
    Click,
    #[prse = "key {}"]
    Key(char),
}

impl Command {
    pub fn run(&self) {
        match self {
            Command::Wait(t) => {
                info!("Found wait {}", t);
            }
            Command::MoveMouse { x, y } => {
                info!("Found mouse move with x = {}, y = {}", x, y);
            }
            Command::Click => {
                info!("Found click");
            }
            Command::Key(c) => {
                info!("Found key {}", c)
            }
        }
    }
}
